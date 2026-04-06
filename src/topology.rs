use std::collections::HashMap;

use crate::device::UsbDevice;

/// A physical USB controller (one or more companion buses sharing a PCI slot).
pub struct PhysicalController<'a> {
    pub pci_slot: String,
    /// Root hubs on this controller, sorted by speed (highest first)
    pub root_hubs: Vec<&'a UsbDevice>,
    /// Maximum speed across all companion buses
    pub max_speed: f64,
    /// Merged physical device tree
    pub children: Vec<PhysicalDevice<'a>>,
}

/// A merged view of a physical USB device across companion buses.
/// If a device exists on both USB 2.0 and USB 3.x, we pick the faster one
/// and note the port's capability.
pub struct PhysicalDevice<'a> {
    /// The actual device (the fastest version if present on multiple buses)
    pub device: &'a UsbDevice,
    /// Maximum speed the port supports (from the companion bus's root hub or parent hub)
    pub port_max_speed: f64,
    /// True if this device only appears on USB 2.0 but the port supports USB 3.x
    pub speed_limited: bool,
    pub children: Vec<PhysicalDevice<'a>>,
}

/// Build the physical topology by merging companion buses.
pub fn build_physical_topology(devices: &[UsbDevice]) -> Vec<PhysicalController<'_>> {
    // Group root hubs by PCI slot
    let mut slot_roots: HashMap<&str, Vec<&UsbDevice>> = HashMap::new();
    for dev in devices.iter().filter(|d| d.is_root_hub()) {
        if let Some(ref slot) = dev.pci_slot {
            slot_roots.entry(slot.as_str()).or_default().push(dev);
        }
    }

    // Build a lookup: bus number -> all non-root devices on that bus
    let mut bus_devices: HashMap<u8, Vec<&UsbDevice>> = HashMap::new();
    for dev in devices.iter().filter(|d| !d.is_root_hub()) {
        bus_devices.entry(dev.bus).or_default().push(dev);
    }

    let mut controllers: Vec<PhysicalController> = Vec::new();

    for (slot, mut roots) in slot_roots {
        roots.sort_by(|a, b| b.speed.partial_cmp(&a.speed).unwrap());
        let max_speed = roots.iter().map(|r| r.speed).fold(0.0_f64, f64::max);

        // Collect all devices across companion buses, keyed by devpath
        // For each devpath, keep the device with the highest speed
        let mut by_devpath: HashMap<&str, &UsbDevice> = HashMap::new();
        // Also track the max speed available at each devpath (from the highest-speed bus)
        let mut devpath_max_speed: HashMap<&str, f64> = HashMap::new();

        for root in &roots {
            if let Some(devs) = bus_devices.get(&root.bus) {
                for dev in devs {
                    // Track which devpaths exist on which speed buses
                    let existing_speed = devpath_max_speed.entry(&dev.devpath).or_insert(0.0);
                    // The port's max speed is the root hub speed (the bus speed)
                    // But for devices behind hubs, it's limited by the parent hub's speed
                    // For now, track that the devpath exists on this bus
                    if root.speed > *existing_speed {
                        *existing_speed = root.speed;
                    }

                    let entry = by_devpath.entry(&dev.devpath);
                    entry
                        .and_modify(|existing| {
                            if dev.speed > existing.speed {
                                *existing = dev;
                            }
                        })
                        .or_insert(dev);
                }
            }
        }

        // Now figure out the actual port max speed for each device.
        // A device's port max speed is limited by its parent hub's speed.
        // We compute this by walking up the devpath.
        let mut port_max_speeds: HashMap<&str, f64> = HashMap::new();

        // Sort devpaths by depth (shortest first) so parents are resolved before children
        let mut devpaths: Vec<&str> = by_devpath.keys().copied().collect();
        devpaths.sort_by_key(|p| p.matches('.').count());

        for devpath in &devpaths {
            // Parent devpath: "2.1" -> "2", "2.2.4" -> "2.2", "2" -> root
            let parent_speed = if let Some(dot_pos) = devpath.rfind('.') {
                let parent_path = &devpath[..dot_pos];
                // Parent's actual speed (what it negotiated) if it's a hub
                // But we want the max speed available at the parent port
                // A hub can't provide more than its own connection speed
                if let Some(parent_dev) = by_devpath.get(parent_path) {
                    // The parent hub's negotiated speed limits what children can get
                    parent_dev.speed
                } else {
                    // Parent exists only on one bus — check what speed bus it's on
                    *devpath_max_speed.get(parent_path).unwrap_or(&max_speed)
                }
            } else {
                // Top-level port — limited by controller max speed
                max_speed
            };

            // The port could support up to the parent's speed,
            // but also check if this devpath exists on a higher-speed bus
            let bus_max = *devpath_max_speed.get(devpath).unwrap_or(&0.0);
            port_max_speeds.insert(devpath, parent_speed.min(bus_max.max(parent_speed)));
        }

        // Build the physical device tree
        let physical_devices =
            build_device_tree(&by_devpath, &port_max_speeds, &devpaths, max_speed);

        controllers.push(PhysicalController {
            pci_slot: slot.to_string(),
            root_hubs: roots,
            max_speed,
            children: physical_devices,
        });
    }

    // Sort controllers by PCI slot for stable output
    controllers.sort_by(|a, b| a.pci_slot.cmp(&b.pci_slot));
    controllers
}

fn build_device_tree<'a>(
    by_devpath: &HashMap<&str, &'a UsbDevice>,
    port_max_speeds: &HashMap<&str, f64>,
    devpaths: &[&str],
    controller_max_speed: f64,
) -> Vec<PhysicalDevice<'a>> {
    // Find top-level devices (no dot in devpath = direct children of root hub)
    let top_level: Vec<&str> = devpaths
        .iter()
        .filter(|p| !p.contains('.'))
        .copied()
        .collect();

    build_children(
        &top_level,
        by_devpath,
        port_max_speeds,
        devpaths,
        controller_max_speed,
    )
}

fn build_children<'a>(
    parent_children_paths: &[&str],
    by_devpath: &HashMap<&str, &'a UsbDevice>,
    port_max_speeds: &HashMap<&str, f64>,
    all_devpaths: &[&str],
    _controller_max_speed: f64,
) -> Vec<PhysicalDevice<'a>> {
    let mut result = Vec::new();

    for &devpath in parent_children_paths {
        let Some(&device) = by_devpath.get(devpath) else {
            continue;
        };

        let port_max = *port_max_speeds.get(devpath).unwrap_or(&device.speed);
        let speed_limited = device.speed < port_max && port_max > 480.0 && device.speed <= 480.0;

        // Find children of this device
        let prefix = format!("{}.", devpath);
        let child_paths: Vec<&str> = all_devpaths
            .iter()
            .filter(|p| {
                p.starts_with(&prefix) && !p[prefix.len()..].contains('.')
            })
            .copied()
            .collect();

        let children = build_children(
            &child_paths,
            by_devpath,
            port_max_speeds,
            all_devpaths,
            _controller_max_speed,
        );

        result.push(PhysicalDevice {
            device,
            port_max_speed: port_max,
            speed_limited,
            children,
        });
    }

    result.sort_by_key(|pd| pd.device.port_number().unwrap_or(0));
    result
}
