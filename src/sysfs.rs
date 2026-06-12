use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::device::{UsbDevice, UsbInterface};
use crate::usb_ids::UsbIds;

const SYSFS_USB_DEVICES: &str = "/sys/bus/usb/devices";

/// Result of a sysfs scan: all devices plus the port peer map.
pub struct Scan {
    pub devices: Vec<UsbDevice>,
    /// Companion port links (both directions), e.g. "usb1-port3" <-> "usb2-port3".
    /// Peered ports are the same physical connector on the USB 2.0 and
    /// USB 3.x side of a controller or hub.
    pub peers: HashMap<String, String>,
}

/// Scan all USB devices and hub port peer links from sysfs.
pub fn scan_devices(usb_ids: &UsbIds) -> Result<Scan, Box<dyn std::error::Error>> {
    let mut devices = Vec::new();
    let mut peers = HashMap::new();

    let entries = fs::read_dir(SYSFS_USB_DEVICES)?;
    // Skip unreadable entries instead of failing the whole scan
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip interface entries (contain ':')
        if name.contains(':') {
            continue;
        }

        let path = entry.path();
        // Resolve symlink to actual path
        let path = fs::canonicalize(&path).unwrap_or(path);

        if let Some(dev) = read_device(&path, &name, usb_ids) {
            devices.push(dev);
        }
        scan_port_peers(&path, &mut peers);
    }

    // Propagate PCI slot from root hubs to all devices on that bus
    let pci_slots: HashMap<u8, String> = devices
        .iter()
        .filter(|d| d.is_root_hub())
        .filter_map(|d| d.serial.as_ref().map(|s| (d.bus, s.clone())))
        .collect();

    for dev in &mut devices {
        dev.pci_slot = pci_slots.get(&dev.bus).cloned();
    }

    Ok(Scan { devices, peers })
}

/// Collect peer links of all hub ports below a device directory.
/// Port directories ("<hub>-port<N>") live inside the hub's interface
/// directory; their "peer" symlink points to the companion-bus port
/// that shares the same physical connector.
fn scan_port_peers(device_path: &Path, peers: &mut HashMap<String, String>) {
    let Ok(entries) = fs::read_dir(device_path) else {
        return;
    };
    for entry in entries.flatten() {
        let iface_name = entry.file_name().to_string_lossy().to_string();
        if !iface_name.contains(':') {
            continue;
        }
        let Ok(port_entries) = fs::read_dir(entry.path()) else {
            continue;
        };
        for port in port_entries.flatten() {
            let port_name = port.file_name().to_string_lossy().to_string();
            if !port_name.contains("-port") {
                continue;
            }
            if let Ok(target) = fs::read_link(port.path().join("peer"))
                && let Some(peer_name) = target.file_name()
            {
                let peer_name = peer_name.to_string_lossy().to_string();
                peers.insert(port_name.clone(), peer_name.clone());
                peers.insert(peer_name, port_name);
            }
        }
    }
}

/// The sysfs name of the port a device hangs off: "2-3.1" sits on port 1
/// of hub "2-3" ("2-3-port1"), top-level "2-4" on port 4 of the root hub
/// ("usb2-port4"). Root hubs have no parent port.
fn parent_port_name(sysfs_name: &str, bus: u8, devpath: &str) -> Option<String> {
    if devpath == "0" {
        return None;
    }
    if let Some((parent, port)) = sysfs_name.rsplit_once('.') {
        Some(format!("{parent}-port{port}"))
    } else {
        let (_, port) = sysfs_name.split_once('-')?;
        Some(format!("usb{bus}-port{port}"))
    }
}

fn read_device(path: &Path, sysfs_name: &str, usb_ids: &UsbIds) -> Option<UsbDevice> {
    let vendor_id = read_hex(path, "idVendor")?;
    let product_id = read_hex(path, "idProduct")?;
    let bus = read_decimal::<u8>(path, "busnum")?;
    let devnum = read_decimal::<u8>(path, "devnum")?;
    let devpath = read_attr(path, "devpath")?;
    let speed: f64 = read_attr(path, "speed")?.parse().ok()?;
    let usb_version = read_attr(path, "version")?;
    let device_class = read_hex_u8(path, "bDeviceClass")?;
    let device_subclass = read_hex_u8(path, "bDeviceSubClass")?;
    let device_protocol = read_hex_u8(path, "bDeviceProtocol")?;
    let num_interfaces = read_decimal::<u8>(path, "bNumInterfaces").unwrap_or(0);

    let manufacturer = read_attr(path, "manufacturer");
    let product = read_attr(path, "product");
    let serial = read_attr(path, "serial");
    let max_power = read_attr(path, "bMaxPower");
    let removable = read_attr(path, "removable");
    let max_children = read_decimal::<u8>(path, "maxchild").filter(|&n| n > 0);

    let vendor_name = usb_ids
        .vendor_name(vendor_id)
        .map(std::string::ToString::to_string);
    let product_name = usb_ids
        .product_name(vendor_id, product_id)
        .map(std::string::ToString::to_string);

    let interfaces = read_interfaces(path, sysfs_name);
    let parent_port = parent_port_name(sysfs_name, bus, &devpath);

    Some(UsbDevice {
        sysfs_name: sysfs_name.to_string(),
        bus,
        devnum,
        devpath,
        vendor_id,
        product_id,
        manufacturer,
        product,
        vendor_name,
        product_name,
        serial,
        speed,
        usb_version,
        device_class,
        device_subclass,
        device_protocol,
        max_power,
        num_interfaces,
        removable,
        max_children,
        interfaces,
        pci_slot: None,
        parent_port,
    })
}

fn read_interfaces(device_path: &Path, sysfs_name: &str) -> Vec<UsbInterface> {
    let mut interfaces = Vec::new();

    // Interface entries are in the same parent directory, named like "5-2.1:1.0"
    // But after canonicalize, they're subdirectories of the device
    let prefix = format!("{sysfs_name}:");

    // Try reading interface dirs from the device directory itself
    let Ok(entries) = fs::read_dir(device_path) else {
        return interfaces;
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(&prefix) {
            continue;
        }

        let iface_path = entry.path();
        if let Some(iface) = read_interface(&iface_path) {
            interfaces.push(iface);
        }
    }

    interfaces.sort_by_key(|i| i.number);
    interfaces
}

fn read_interface(path: &Path) -> Option<UsbInterface> {
    let number = read_hex_u8(path, "bInterfaceNumber")?;
    let class = read_hex_u8(path, "bInterfaceClass")?;
    let subclass = read_hex_u8(path, "bInterfaceSubClass")?;
    let protocol = read_hex_u8(path, "bInterfaceProtocol")?;
    let num_endpoints = read_hex_u8(path, "bNumEndpoints").unwrap_or(0);
    let driver = read_driver(path);

    Some(UsbInterface {
        number,
        class,
        subclass,
        protocol,
        num_endpoints,
        driver,
    })
}

fn read_driver(path: &Path) -> Option<String> {
    let driver_link = path.join("driver");
    fs::read_link(&driver_link)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
}

// --- sysfs attribute helpers ---

fn read_attr(path: &Path, name: &str) -> Option<String> {
    let content = fs::read_to_string(path.join(name)).ok()?;
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn read_hex(path: &Path, name: &str) -> Option<u16> {
    let s = read_attr(path, name)?;
    u16::from_str_radix(&s, 16).ok()
}

fn read_hex_u8(path: &Path, name: &str) -> Option<u8> {
    let s = read_attr(path, name)?;
    u8::from_str_radix(&s, 16).ok()
}

fn read_decimal<T: std::str::FromStr>(path: &Path, name: &str) -> Option<T> {
    let s = read_attr(path, name)?;
    s.parse().ok()
}
