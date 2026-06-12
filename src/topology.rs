use std::collections::HashMap;

use crate::device::UsbDevice;

/// A physical USB controller (one or more companion buses).
pub struct PhysicalController<'a> {
    /// PCI slot of the controller, if known (from the root hub serial)
    pub pci_slot: Option<String>,
    /// Root hubs on this controller, sorted by speed (highest first)
    pub root_hubs: Vec<&'a UsbDevice>,
    /// Maximum speed across all companion buses
    pub max_speed: f64,
    /// Merged physical device tree
    pub children: Vec<PhysicalDevice<'a>>,
}

/// A physical USB device, merged across companion buses.
/// USB 3.x hubs enumerate twice (once per bus); both instances sit on
/// peered ports and are folded into one node, keeping the faster one.
pub struct PhysicalDevice<'a> {
    /// The device (the fastest instance if it enumerates on multiple buses)
    pub device: &'a UsbDevice,
    /// Maximum speed the physical port supports
    pub port_max_speed: f64,
    /// True if the device runs at USB 2.0 speed on a port wired for USB 3.x
    pub speed_limited: bool,
    pub children: Vec<PhysicalDevice<'a>>,
}

/// Hub instances acting as one physical parent: sysfs name -> negotiated speed.
/// At the root level these are the controller's root hubs ("usb1", "usb2");
/// below that, the (up to two) instances of a merged hub.
type ParentInstances<'a> = Vec<(&'a str, f64)>;

/// Devices sharing one physical port, with the port's wired capability.
struct Slot<'a> {
    devices: Vec<&'a UsbDevice>,
    max_speed: f64,
}

/// Build the physical topology by merging companion buses.
///
/// Physical identity comes from the sysfs port peer links: peered ports
/// ("usb1-port3" <-> "usb2-port3") are the USB 2.0 and USB 3.x side of the
/// same connector. Ports without a peer are single-signal (e.g. internal
/// USB 2.0-only headers) and are never merged or reported as speed-limited
/// against the faster bus.
pub fn build_physical_topology<'a>(
    devices: &'a [UsbDevice],
    peers: &HashMap<String, String>,
) -> Vec<PhysicalController<'a>> {
    // Devices attached to each hub instance: owner sysfs name -> children
    let mut children_by_owner: HashMap<&str, Vec<&UsbDevice>> = HashMap::new();
    for dev in devices {
        if let Some(owner) = dev.parent_port.as_deref().and_then(port_owner) {
            children_by_owner.entry(owner).or_default().push(dev);
        }
    }

    let controller_groups = group_root_hubs(devices, peers);

    let mut controllers: Vec<PhysicalController> = controller_groups
        .into_iter()
        .map(|mut roots| {
            roots.sort_by(|a, b| b.speed.total_cmp(&a.speed));
            let max_speed = roots.iter().map(|r| r.speed).fold(0.0_f64, f64::max);
            let instances: ParentInstances = roots
                .iter()
                .map(|r| (r.sysfs_name.as_str(), r.speed))
                .collect();
            let children = build_children(&instances, &children_by_owner, peers);

            PhysicalController {
                pci_slot: roots.iter().find_map(|r| r.pci_slot.clone()),
                root_hubs: roots,
                max_speed,
                children,
            }
        })
        .collect();

    // Sort controllers by their lowest bus number for stable output
    controllers.sort_by_key(|c| c.root_hubs.iter().map(|r| r.bus).min().unwrap_or(0));
    controllers
}

/// Group root hubs into physical controllers. Primary signal: peer links
/// between their ports. Fallback for kernels without port peering: a shared
/// PCI slot. Buses without either become their own controller.
fn group_root_hubs<'a>(
    devices: &'a [UsbDevice],
    peers: &HashMap<String, String>,
) -> Vec<Vec<&'a UsbDevice>> {
    let roots: Vec<&UsbDevice> = devices.iter().filter(|d| d.is_root_hub()).collect();
    let index_of: HashMap<&str, usize> = roots
        .iter()
        .enumerate()
        .map(|(i, r)| (r.sysfs_name.as_str(), i))
        .collect();

    let mut uf = UnionFind::new(roots.len());

    // Union buses whose root ports are peered
    for (port, peer) in peers {
        if let (Some(a), Some(b)) = (
            port_owner(port).and_then(|o| index_of.get(o)),
            port_owner(peer).and_then(|o| index_of.get(o)),
        ) {
            uf.union(*a, *b);
        }
    }

    // Union buses sharing a PCI slot
    let mut by_slot: HashMap<&str, usize> = HashMap::new();
    for (i, root) in roots.iter().enumerate() {
        if let Some(slot) = root.pci_slot.as_deref() {
            if let Some(&first) = by_slot.get(slot) {
                uf.union(first, i);
            } else {
                by_slot.insert(slot, i);
            }
        }
    }

    let mut groups: HashMap<usize, Vec<&UsbDevice>> = HashMap::new();
    for (i, root) in roots.iter().enumerate() {
        groups.entry(uf.find(i)).or_default().push(root);
    }
    groups.into_values().collect()
}

/// Build the merged children of one physical parent (a set of hub instances).
fn build_children<'a>(
    instances: &ParentInstances<'a>,
    children_by_owner: &HashMap<&'a str, Vec<&'a UsbDevice>>,
    peers: &HashMap<String, String>,
) -> Vec<PhysicalDevice<'a>> {
    let instance_speed: HashMap<&str, f64> = instances.iter().copied().collect();

    // The instance (within this parent) owning a port's peer, if any.
    // A peer outside the parent cannot happen structurally, but be safe.
    let in_group_peer = |port: &str| -> Option<&str> {
        let peer = peers.get(port)?.as_str();
        port_owner(peer).filter(|o| instance_speed.contains_key(o))?;
        Some(peer)
    };

    // Group attached devices by physical port. Peered ports are one
    // connector, so their devices (a hub's two instances) merge.
    let mut slots: HashMap<&str, Slot> = HashMap::new();

    for (name, _) in instances {
        for dev in children_by_owner.get(name).into_iter().flatten() {
            let Some(port) = dev.parent_port.as_deref() else {
                continue;
            };
            let peer = in_group_peer(port);
            // Canonical key: the lexicographically smaller of the pair
            let key = peer.map_or(port, |p| p.min(port));

            // The port's capability is the fastest parent side it is wired to
            let side_speed = |p: &str| {
                port_owner(p)
                    .and_then(|o| instance_speed.get(o))
                    .copied()
                    .unwrap_or(0.0)
            };
            let port_speed = peer.map_or(0.0_f64, &side_speed).max(side_speed(port));

            let slot = slots.entry(key).or_insert(Slot {
                devices: Vec::new(),
                max_speed: 0.0,
            });
            slot.devices.push(dev);
            slot.max_speed = slot.max_speed.max(port_speed);
        }
    }

    let mut result: Vec<PhysicalDevice> = slots
        .into_values()
        .map(|mut slot| {
            slot.devices.sort_by(|a, b| b.speed.total_cmp(&a.speed));
            let device = slot.devices[0];

            // Recurse with all instances of this (possibly merged) device
            let child_instances: ParentInstances = slot
                .devices
                .iter()
                .map(|d| (d.sysfs_name.as_str(), d.speed))
                .collect();
            let children = build_children(&child_instances, children_by_owner, peers);

            let speed_limited =
                device.speed < slot.max_speed && slot.max_speed > 480.0 && device.speed <= 480.0;

            PhysicalDevice {
                device,
                port_max_speed: slot.max_speed,
                speed_limited,
                children,
            }
        })
        .collect();

    result.sort_by_key(|pd| pd.device.port_number().unwrap_or(0));
    result
}

/// The hub instance a port belongs to: "2-3-port1" -> "2-3", "usb1-port6" -> "usb1".
fn port_owner(port: &str) -> Option<&str> {
    port.rsplit_once("-port").map(|(owner, _)| owner)
}

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }

    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[ra] = rb;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(
        sysfs_name: &str,
        bus: u8,
        devpath: &str,
        speed: f64,
        parent_port: Option<&str>,
        pci_slot: Option<&str>,
    ) -> UsbDevice {
        UsbDevice {
            sysfs_name: sysfs_name.to_string(),
            bus,
            devnum: 1,
            devpath: devpath.to_string(),
            vendor_id: 0x1234,
            product_id: 0x5678,
            manufacturer: None,
            product: None,
            vendor_name: None,
            product_name: None,
            serial: None,
            speed,
            usb_version: "2.00".to_string(),
            device_class: 0,
            device_subclass: 0,
            device_protocol: 0,
            max_power: None,
            num_interfaces: 0,
            removable: None,
            max_children: None,
            interfaces: Vec::new(),
            pci_slot: pci_slot.map(str::to_string),
            parent_port: parent_port.map(str::to_string),
        }
    }

    fn peers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for (a, b) in pairs {
            map.insert((*a).to_string(), (*b).to_string());
            map.insert((*b).to_string(), (*a).to_string());
        }
        map
    }

    #[test]
    fn unpeered_port_is_not_speed_limited() {
        // Controller: usb1 (480) + usb2 (10000), only port 1 is dual-wired.
        // The device on usb1-port2 sits on a USB 2.0-only port and must not
        // be flagged as limited against the 10 Gbps bus.
        let devices = vec![
            dev("usb1", 1, "0", 480.0, None, Some("slot")),
            dev("usb2", 2, "0", 10000.0, None, Some("slot")),
            dev("1-1", 1, "1", 480.0, Some("usb1-port1"), Some("slot")),
            dev("1-2", 1, "2", 12.0, Some("usb1-port2"), Some("slot")),
        ];
        let peers = peers(&[("usb1-port1", "usb2-port1")]);

        let controllers = build_physical_topology(&devices, &peers);
        assert_eq!(controllers.len(), 1);
        let children = &controllers[0].children;
        assert_eq!(children.len(), 2);

        let on_dual = &children[0]; // port 1
        assert!(on_dual.speed_limited);
        assert!((on_dual.port_max_speed - 10000.0).abs() < f64::EPSILON);

        let on_usb2_only = &children[1]; // port 2
        assert!(!on_usb2_only.speed_limited);
        assert!((on_usb2_only.port_max_speed - 480.0).abs() < f64::EPSILON);
    }

    #[test]
    fn devices_on_unpeered_ports_with_same_number_do_not_merge() {
        // Regression: the old devpath heuristic collapsed devices at the
        // same port number across companion buses, hiding one of them.
        let devices = vec![
            dev("usb1", 1, "0", 480.0, None, Some("slot")),
            dev("usb2", 2, "0", 5000.0, None, Some("slot")),
            dev("1-2", 1, "2", 480.0, Some("usb1-port2"), Some("slot")),
            dev("2-2", 2, "2", 5000.0, Some("usb2-port2"), Some("slot")),
        ];
        // Ports 2 of both buses are distinct physical connectors
        let peers = peers(&[("usb1-port1", "usb2-port1")]);

        let controllers = build_physical_topology(&devices, &peers);
        assert_eq!(controllers.len(), 1);
        assert_eq!(controllers[0].children.len(), 2);
    }

    #[test]
    fn hub_instances_on_peered_ports_merge_to_fastest() {
        // A USB 3.x hub enumerates on both buses; its instances sit on
        // peered ports and fold into one node showing the faster instance.
        let devices = vec![
            dev("usb1", 1, "0", 480.0, None, Some("slot")),
            dev("usb2", 2, "0", 5000.0, None, Some("slot")),
            dev("1-3", 1, "3", 480.0, Some("usb1-port3"), Some("slot")),
            dev("2-3", 2, "3", 5000.0, Some("usb2-port3"), Some("slot")),
            // Device on the hub's USB 2.0 side, on a dual-wired hub port
            dev("1-3.1", 1, "3.1", 480.0, Some("1-3-port1"), Some("slot")),
        ];
        let peers = peers(&[("usb1-port3", "usb2-port3"), ("1-3-port1", "2-3-port1")]);

        let controllers = build_physical_topology(&devices, &peers);
        assert_eq!(controllers.len(), 1);
        assert_eq!(controllers[0].children.len(), 1);

        let hub = &controllers[0].children[0];
        assert_eq!(hub.device.sysfs_name, "2-3");
        assert!(!hub.speed_limited);

        // The child hangs off the merged hub and its port supports 5 Gbps
        assert_eq!(hub.children.len(), 1);
        let child = &hub.children[0];
        assert_eq!(child.device.sysfs_name, "1-3.1");
        assert!(child.speed_limited);
        assert!((child.port_max_speed - 5000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn root_hub_without_pci_slot_still_forms_controller() {
        // Regression: buses whose root hub has no PCI slot were dropped
        // from the tree entirely.
        let devices = vec![
            dev("usb1", 1, "0", 480.0, None, None),
            dev("1-1", 1, "1", 12.0, Some("usb1-port1"), None),
        ];
        let controllers = build_physical_topology(&devices, &HashMap::new());
        assert_eq!(controllers.len(), 1);
        assert!(controllers[0].pci_slot.is_none());
        assert_eq!(controllers[0].children.len(), 1);
    }

    #[test]
    fn pci_slot_groups_buses_when_peer_links_are_missing() {
        // Fallback for kernels without port peering
        let devices = vec![
            dev("usb1", 1, "0", 480.0, None, Some("slot")),
            dev("usb2", 2, "0", 5000.0, None, Some("slot")),
            dev("usb3", 3, "0", 480.0, None, Some("other")),
        ];
        let controllers = build_physical_topology(&devices, &HashMap::new());
        assert_eq!(controllers.len(), 2);
        assert_eq!(controllers[0].root_hubs.len(), 2);
        assert_eq!(controllers[1].root_hubs.len(), 1);
    }
}
