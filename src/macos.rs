//! macOS USB backend.
//!
//! Reads the device tree from `system_profiler SPUSBHostDataType -json` and
//! maps it onto the same neutral [`Scan`] the Linux sysfs backend produces.
//! macOS/`IOKit` already presents the merged physical topology, so there are no
//! companion buses to merge: [`Scan::peers`] is always empty and `topology.rs`
//! builds a straight single-signal tree.
//!
//! Physical position comes from the Apple location ID (e.g. `0x00143400`): the
//! top byte is the host controller, each following nibble (MSB first, until the
//! first zero) is a hub port. From that we synthesise the same sysfs-shaped
//! names (`usb1`, `1-2.3`, `1-2-port3`) that `topology.rs` expects, so the tree
//! builder is shared verbatim.
//!
//! `SPUSBHostDataType` does not expose USB interfaces, per-interface drivers, or
//! device class codes, so those fields stay empty. See `plans/macos-port.md` in
//! the sidecar for the ioreg/`IOKit` upgrade path that would fill them.

use std::collections::HashMap;
use std::process::Command;

use serde_json::Value;

use crate::device::{Scan, UsbDevice};
use crate::usb_ids::UsbIds;

/// Scan USB devices via `system_profiler` and map them onto a neutral [`Scan`].
pub fn scan_devices(usb_ids: &UsbIds) -> Result<Scan, Box<dyn std::error::Error>> {
    let output = Command::new("system_profiler")
        .args(["SPUSBHostDataType", "-json"])
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "system_profiler exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    let root: Value = serde_json::from_slice(&output.stdout)?;
    Ok(parse_scan(&root, usb_ids))
}

/// Turn a parsed `SPUSBHostDataType` document into a [`Scan`]. Split out from
/// [`scan_devices`] so it can be tested against a fixture without a subprocess.
fn parse_scan(root: &Value, usb_ids: &UsbIds) -> Scan {
    let Some(items) = root.get("SPUSBHostDataType").and_then(Value::as_array) else {
        return empty_scan();
    };

    // Host-controller name per bus byte: the top-level entries whose location
    // ID carries no port chain are the controllers themselves.
    let mut controller_names: HashMap<u8, String> = HashMap::new();
    for item in items {
        let Some((bus_byte, chain)) = location_of(item) else {
            continue;
        };
        if chain.is_empty()
            && let Some(name) = item.get("_name").and_then(Value::as_str)
        {
            controller_names
                .entry(bus_byte)
                .or_insert_with(|| name.to_string());
        }
    }

    // Every real device node (one carrying a vendor ID), with its decoded
    // (bus byte, port chain). Root controllers have an empty chain and drop out.
    let mut nodes: Vec<&Value> = Vec::new();
    collect_devices(items, &mut nodes);
    let decoded: Vec<(u8, Vec<u8>, &Value)> = nodes
        .into_iter()
        .filter_map(|v| {
            let (bus_byte, chain) = location_of(v)?;
            (!chain.is_empty()).then_some((bus_byte, chain, v))
        })
        .collect();

    // Assign synthetic bus indices 1..=N to the populated buses (empty buses
    // never get a root hub, which keeps stray "USB 4.0 Bus" headers out).
    let mut bus_bytes: Vec<u8> = decoded.iter().map(|(b, _, _)| *b).collect();
    bus_bytes.sort_unstable();
    bus_bytes.dedup();
    let bus_index: HashMap<u8, u8> = bus_bytes
        .iter()
        .enumerate()
        .map(|(i, &b)| (b, u8::try_from(i + 1).unwrap_or(u8::MAX)))
        .collect();

    let mut devices: Vec<UsbDevice> = Vec::new();

    // One root hub per populated bus.
    for &bus_byte in &bus_bytes {
        let idx = bus_index[&bus_byte];
        let name = controller_names
            .get(&bus_byte)
            .cloned()
            .unwrap_or_else(|| "USB Host Controller".to_string());
        // Nominal speed from the bus name, but never below the fastest device
        // actually enumerated on it.
        let child_max = decoded
            .iter()
            .filter(|(b, _, _)| *b == bus_byte)
            .map(|(_, _, v)| link_speed(v))
            .fold(0.0_f64, f64::max);
        let speed = bus_name_speed(&name).max(child_max);
        devices.push(root_hub(idx, name, speed));
    }

    // Devices, numbered sequentially per bus for a stable devnum in list mode.
    let mut devnum: HashMap<u8, u8> = HashMap::new();
    for (bus_byte, chain, value) in &decoded {
        let idx = bus_index[bus_byte];
        let n = devnum.entry(idx).or_insert(0);
        *n = n.saturating_add(1);
        if let Some(dev) = build_device(value, idx, chain, *n, usb_ids) {
            devices.push(dev);
        }
    }

    Scan {
        devices,
        peers: HashMap::new(),
    }
}

fn empty_scan() -> Scan {
    Scan {
        devices: Vec::new(),
        peers: HashMap::new(),
    }
}

/// Recursively collect every node that looks like a real USB device.
fn collect_devices<'a>(items: &'a [Value], out: &mut Vec<&'a Value>) {
    for item in items {
        if item.get("USBDeviceKeyVendorID").is_some() {
            out.push(item);
        }
        if let Some(children) = item.get("_items").and_then(Value::as_array) {
            collect_devices(children, out);
        }
    }
}

/// Build a device node. Fields absent from `SPUSBHostDataType` (class codes,
/// interfaces, power) stay empty; the tree view does not need them.
fn build_device(
    v: &Value,
    bus: u8,
    chain: &[u8],
    devnum: u8,
    usb_ids: &UsbIds,
) -> Option<UsbDevice> {
    let vendor_id = parse_hex_u16(v.get("USBDeviceKeyVendorID"))?;
    let product_id = parse_hex_u16(v.get("USBDeviceKeyProductID"))?;

    let devpath = join_chain(chain);
    let sysfs_name = format!("{bus}-{devpath}");
    let parent_port = synth_parent_port(bus, chain);
    let speed = link_speed(v);

    let product = string_field(v, "_name");
    let mut manufacturer = string_field(v, "USBDeviceKeyVendorName");
    // Apple's product name often already embeds the vendor ("SMSL USB AUDIO",
    // "CORSAIR K70 …"); drop the redundant manufacturer prefix in that case so
    // `display_name` does not repeat it.
    if let (Some(m), Some(p)) = (&manufacturer, &product)
        && p.to_lowercase().contains(&m.to_lowercase())
    {
        manufacturer = None;
    }
    let serial = string_field(v, "USBDeviceKeySerialNumber")
        .filter(|s| !s.eq_ignore_ascii_case("Not Provided"));
    let removable = v
        .get("USBKeyHardwareType")
        .and_then(Value::as_str)
        .map(|t| {
            if t.eq_ignore_ascii_case("Removable") {
                "removable"
            } else {
                "fixed"
            }
            .to_string()
        });

    // usb.ids may still be present (e.g. via Homebrew's `usbutils`), so fill the
    // database names too; `display_name` prefers the device's own strings.
    let vendor_name = usb_ids.vendor_name(vendor_id).map(str::to_string);
    let product_name = usb_ids
        .product_name(vendor_id, product_id)
        .map(str::to_string);

    Some(UsbDevice {
        sysfs_name,
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
        usb_version: coarse_usb_version(speed),
        device_class: 0,
        device_subclass: 0,
        device_protocol: 0,
        max_power: None,
        num_interfaces: 0,
        removable,
        max_children: None,
        interfaces: Vec::new(),
        pci_slot: None,
        parent_port,
    })
}

fn root_hub(bus: u8, name: String, speed: f64) -> UsbDevice {
    UsbDevice {
        sysfs_name: format!("usb{bus}"),
        bus,
        devnum: 0,
        devpath: "0".to_string(),
        vendor_id: 0,
        product_id: 0,
        manufacturer: None,
        product: Some(name),
        vendor_name: None,
        product_name: None,
        serial: None,
        speed,
        usb_version: coarse_usb_version(speed),
        device_class: 0x09, // hub
        device_subclass: 0,
        device_protocol: 0,
        max_power: None,
        num_interfaces: 0,
        removable: None,
        max_children: None,
        interfaces: Vec::new(),
        pci_slot: None,
        parent_port: None,
    }
}

// --- field parsing ---

fn string_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Decode an Apple location ID ("0x00143400") into (bus byte, port chain).
fn decode_location(loc: &str) -> Option<(u8, Vec<u8>)> {
    let trimmed = loc.trim();
    let hex = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    let raw = u32::from_str_radix(hex, 16).ok()?;
    let bus = u8::try_from(raw >> 24).unwrap_or(0);
    let mut chain = Vec::new();
    // Nibbles below the bus byte, most significant first, until zero padding.
    for shift in (0..24).step_by(4).rev() {
        let nibble = u8::try_from((raw >> shift) & 0xf).unwrap_or(0);
        if nibble == 0 {
            break;
        }
        chain.push(nibble);
    }
    Some((bus, chain))
}

fn location_of(v: &Value) -> Option<(u8, Vec<u8>)> {
    decode_location(v.get("USBKeyLocationID")?.as_str()?)
}

fn join_chain(chain: &[u8]) -> String {
    chain
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

/// The sysfs-style parent port for a device at `chain`. Mirrors what
/// `topology::port_owner` expects: `usb1-port3` under a root, `1-2.3-port4`
/// under a nested hub.
fn synth_parent_port(bus: u8, chain: &[u8]) -> Option<String> {
    let (last, parents) = chain.split_last()?;
    if parents.is_empty() {
        Some(format!("usb{bus}-port{last}"))
    } else {
        Some(format!("{bus}-{}-port{last}", join_chain(parents)))
    }
}

fn parse_hex_u16(v: Option<&Value>) -> Option<u16> {
    let s = v?.as_str()?.trim();
    let hex = s.strip_prefix("0x").unwrap_or(s);
    u16::from_str_radix(hex, 16).ok()
}

/// Parse `USBDeviceKeyLinkSpeed` ("480 Mb/s", "5 Gb/s") into Mbps.
fn link_speed(v: &Value) -> f64 {
    let Some(s) = v.get("USBDeviceKeyLinkSpeed").and_then(Value::as_str) else {
        return 0.0;
    };
    let s = s.trim();
    let (num, mult) = if let Some(n) = s.strip_suffix("Gb/s") {
        (n, 1000.0)
    } else if let Some(n) = s.strip_suffix("Mb/s") {
        (n, 1.0)
    } else {
        return 0.0;
    };
    num.trim().parse::<f64>().map_or(0.0, |x| x * mult)
}

/// Nominal maximum speed (Mbps) a host controller advertises, from its bus name.
fn bus_name_speed(name: &str) -> f64 {
    let n = name.to_ascii_lowercase();
    if n.contains("usb 4") {
        40000.0
    } else if n.contains("3.2") || n.contains("3.1") {
        10000.0
    } else if n.contains("usb 3") {
        5000.0
    } else if n.contains("usb 2") {
        480.0
    } else if n.contains("usb 1") {
        12.0
    } else {
        0.0
    }
}

/// `SPUSBHostDataType` carries no bcdUSB, so approximate the spec version from
/// the negotiated link speed. Only shown under `-v`.
fn coarse_usb_version(speed: f64) -> String {
    if speed >= 5000.0 { "3.x" } else { "2.x" }.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology::build_physical_topology;

    // One controller, a 5 Gbps hub on root port 2, a 480 Mbps keyboard on the
    // hub's port 1 (speed-limited against the hub).
    const FIXTURE: &str = r#"{
      "SPUSBHostDataType": [
        {
          "_name": "USB 3.1 Bus",
          "USBKeyLocationID": "0x00000000",
          "_items": [
            {
              "_name": "USB3.0 Hub",
              "USBDeviceKeyVendorID": "0x05e3",
              "USBDeviceKeyProductID": "0x0610",
              "USBDeviceKeyVendorName": "GenesysLogic",
              "USBDeviceKeyLinkSpeed": "5 Gb/s",
              "USBKeyLocationID": "0x00200000",
              "USBDeviceKeySerialNumber": "Not Provided",
              "USBKeyHardwareType": "Removable",
              "_items": [
                {
                  "_name": "Gaming Keyboard",
                  "USBDeviceKeyVendorID": "0x1b1c",
                  "USBDeviceKeyProductID": "0x1bc0",
                  "USBDeviceKeyVendorName": "Corsair",
                  "USBDeviceKeyLinkSpeed": "480 Mb/s",
                  "USBKeyLocationID": "0x00210000",
                  "USBKeyHardwareType": "Non-removable"
                }
              ]
            }
          ]
        },
        {
          "_name": "USB 4.0 Bus",
          "USBKeyLocationID": "0x08000000"
        }
      ]
    }"#;

    #[test]
    fn decode_location_extracts_bus_and_port_chain() {
        assert_eq!(
            decode_location("0x00143400"),
            Some((0x00, vec![1, 4, 3, 4]))
        );
        assert_eq!(decode_location("0x00100000"), Some((0x00, vec![1])));
        assert_eq!(decode_location("0x08000000"), Some((0x08, vec![])));
        assert_eq!(decode_location("0x02120000"), Some((0x02, vec![1, 2])));
        assert_eq!(decode_location("garbage"), None);
    }

    #[test]
    fn synth_parent_port_matches_sysfs_shape() {
        assert_eq!(synth_parent_port(1, &[2]).as_deref(), Some("usb1-port2"));
        assert_eq!(synth_parent_port(1, &[2, 1]).as_deref(), Some("1-2-port1"));
        assert_eq!(
            synth_parent_port(1, &[2, 3, 4]).as_deref(),
            Some("1-2.3-port4")
        );
        assert_eq!(synth_parent_port(1, &[]), None);
    }

    #[test]
    fn link_speed_parses_units() {
        let mk = |s: &str| serde_json::json!({ "USBDeviceKeyLinkSpeed": s });
        assert!((link_speed(&mk("480 Mb/s")) - 480.0).abs() < f64::EPSILON);
        assert!((link_speed(&mk("5 Gb/s")) - 5000.0).abs() < f64::EPSILON);
        assert!((link_speed(&mk("10 Gb/s")) - 10000.0).abs() < f64::EPSILON);
        assert!((link_speed(&serde_json::json!({})) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_scan_builds_flat_devices() {
        let root: Value = serde_json::from_str(FIXTURE).unwrap();
        let scan = parse_scan(&root, &UsbIds::empty());

        // Root hub + hub + keyboard; the empty "USB 4.0 Bus" gets no root hub.
        assert_eq!(scan.devices.len(), 3);
        assert!(scan.peers.is_empty());

        let root_hub = scan.devices.iter().find(|d| d.is_root_hub()).unwrap();
        assert_eq!(root_hub.bus, 1);
        assert_eq!(root_hub.product.as_deref(), Some("USB 3.1 Bus"));
        // Bus name says 10 Gbps, which beats the fastest child (5 Gbps).
        assert!((root_hub.speed - 10000.0).abs() < f64::EPSILON);

        let hub = scan
            .devices
            .iter()
            .find(|d| d.product.as_deref() == Some("USB3.0 Hub"))
            .unwrap();
        assert_eq!(hub.sysfs_name, "1-2");
        assert_eq!(hub.parent_port.as_deref(), Some("usb1-port2"));
        assert_eq!(hub.serial, None); // "Not Provided" is dropped
        assert_eq!(hub.removable.as_deref(), Some("removable"));

        let kbd = scan
            .devices
            .iter()
            .find(|d| d.product.as_deref() == Some("Gaming Keyboard"))
            .unwrap();
        assert_eq!(kbd.sysfs_name, "1-2.1");
        assert_eq!(kbd.parent_port.as_deref(), Some("1-2-port1"));
        assert_eq!(kbd.vendor_id, 0x1b1c);
        assert_eq!(kbd.removable.as_deref(), Some("fixed"));
    }

    #[test]
    fn parse_scan_feeds_topology_with_speed_warning() {
        let root: Value = serde_json::from_str(FIXTURE).unwrap();
        let scan = parse_scan(&root, &UsbIds::empty());

        let controllers = build_physical_topology(&scan.devices, &scan.peers);
        assert_eq!(controllers.len(), 1);

        let hub = &controllers[0].children[0];
        assert_eq!(hub.device.product.as_deref(), Some("USB3.0 Hub"));
        assert!(!hub.speed_limited);

        let kbd = &hub.children[0];
        assert_eq!(kbd.device.product.as_deref(), Some("Gaming Keyboard"));
        assert!(kbd.speed_limited);
        assert!((kbd.port_max_speed - 5000.0).abs() < f64::EPSILON);
    }
}
