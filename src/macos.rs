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
//! `SPUSBHostDataType` does not expose USB interfaces, per-interface drivers,
//! device class codes, the descriptor's USB version, or how a port is wired.
//! Those fields are left `None` — never zeroed or emptied, so downstream can
//! tell "not available here" from "the device has none" — and
//! [`UsbDevice::port_capability_known`] is set conservatively. Reading `IOKit`
//! directly (via `ioreg`/`IOUSBHost`) would supply all of them; that is the
//! upgrade path if the degraded fields ever matter.

use std::collections::{BTreeSet, HashMap};
use std::process::Command;

use serde_json::Value;

use crate::device::{ClassCode, Scan, UsbDevice, port_name, sanitize};
use crate::usb_ids::UsbIds;

/// Absolute path on purpose: this is a fixed system tool, and resolving it via
/// `PATH` would let the caller's environment decide what we execute.
const SYSTEM_PROFILER: &str = "/usr/sbin/system_profiler";

/// Scan USB devices via `system_profiler` and map them onto a neutral [`Scan`].
pub fn scan_devices(usb_ids: &UsbIds) -> Result<Scan, Box<dyn std::error::Error>> {
    let output = Command::new(SYSTEM_PROFILER)
        .args(["SPUSBHostDataType", "-json"])
        .output()
        .map_err(|e| format!("cannot run {SYSTEM_PROFILER}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{SYSTEM_PROFILER} exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    let root: Value = serde_json::from_slice(&output.stdout)?;
    parse_scan(&root, usb_ids)
}

/// Turn a parsed `SPUSBHostDataType` document into a [`Scan`]. Split out from
/// [`scan_devices`] so it can be tested against a fixture without a subprocess.
fn parse_scan(root: &Value, usb_ids: &UsbIds) -> Result<Scan, Box<dyn std::error::Error>> {
    // A missing key means the output is not what we expect (a renamed data
    // type in a future macOS). Reporting that as "no devices" would be
    // indistinguishable from an empty machine, so it is an error.
    let Some(items) = root.get("SPUSBHostDataType").and_then(Value::as_array) else {
        return Err(
            format!("unexpected {SYSTEM_PROFILER} output: no SPUSBHostDataType array").into(),
        );
    };

    // Host-controller name per bus byte: the top-level entries whose location
    // ID carries no port chain are the controllers themselves.
    let mut controller_names: HashMap<u8, String> = HashMap::new();
    let mut controller_bytes: BTreeSet<u8> = BTreeSet::new();
    for item in items {
        let Some((bus_byte, chain)) = location_of(item) else {
            continue;
        };
        if !chain.is_empty() {
            continue;
        }
        controller_bytes.insert(bus_byte);
        if let Some(name) = item.get("_name").and_then(Value::as_str) {
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

    // Assign synthetic bus indices 1..=N over *every* host controller the
    // system reports, not just the populated ones. Numbering only the
    // populated buses would renumber everything above a so-far empty bus as
    // soon as a device is plugged into it — `--bus 2` would then silently
    // mean a different controller than a moment before.
    let mut bus_bytes: BTreeSet<u8> = controller_bytes;
    bus_bytes.extend(decoded.iter().map(|(b, _, _)| *b));
    let bus_index: HashMap<u8, u8> = bus_bytes
        .iter()
        .enumerate()
        .map(|(i, &b)| (b, u8::try_from(i + 1).unwrap_or(u8::MAX)))
        .collect();

    // Buses that actually carry devices; empty ones get no root hub, which
    // keeps stray "USB 4.0 Bus" headers out of the tree.
    let populated: BTreeSet<u8> = decoded.iter().map(|(b, _, _)| *b).collect();

    let mut devices: Vec<UsbDevice> = Vec::new();

    // One root hub per populated bus.
    for &bus_byte in &populated {
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

    Ok(Scan {
        devices,
        peers: HashMap::new(),
    })
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
    let node_key = format!("{bus}-{devpath}");
    let parent_port = synth_parent_port(bus, chain);
    let speed = link_speed(v);

    // Apple's product name often already embeds the vendor ("SMSL USB AUDIO",
    // "CORSAIR K70 …"). Both names are reported as-is here; suppressing the
    // redundant one is `display_name`'s job, and it does it for both backends.
    let product = string_field(v, "_name");
    let manufacturer = string_field(v, "USBDeviceKeyVendorName");
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

    // `SPUSBHostDataType` says nothing about how a port is wired. For a
    // removable (user-facing) port on a Mac, assuming the controller's speed
    // is sound — those are SuperSpeed connectors. Internal headers are
    // frequently USB 2.0-only, so a throttling verdict there would be a false
    // positive; withhold it rather than guess.
    let port_capability_known = removable.as_deref() == Some("removable");

    // Kept for a single code path with the Linux backend; macOS has no usb.ids
    // to load, so these are `None` in practice. A device that reports no name of
    // its own simply shows its ID — see `usb_ids::USB_IDS_PATHS`.
    let vendor_name = usb_ids.vendor_name(vendor_id).map(str::to_string);
    let product_name = usb_ids
        .product_name(vendor_id, product_id)
        .map(str::to_string);

    Some(UsbDevice {
        node_key,
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
        // Not reported by this source — see the module doc. `None` rather than
        // a zero or an empty list, so consumers can tell "not available" from
        // "the device has none".
        usb_version: None,
        device_class: None,
        max_power: None,
        num_interfaces: None,
        removable,
        max_children: None,
        interfaces: None,
        pci_slot: None,
        parent_port,
        port_capability_known,
    })
}

fn root_hub(bus: u8, name: String, speed: f64) -> UsbDevice {
    UsbDevice {
        node_key: format!("usb{bus}"),
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
        usb_version: None,
        // Synthesised node, but a root hub is a hub by definition
        device_class: Some(ClassCode {
            class: 0x09,
            subclass: 0,
            protocol: 0,
        }),
        max_power: None,
        num_interfaces: None,
        removable: None,
        max_children: None,
        interfaces: None,
        pci_slot: None,
        parent_port: None,
        // Root hubs have no parent port; the flag is irrelevant for them.
        port_capability_known: false,
    }
}

// --- field parsing ---

/// Read a string field, stripped of control characters — these come from the
/// device's own descriptors and would otherwise be able to inject terminal
/// escape sequences into our output.
fn string_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(sanitize)
        .filter(|s| !s.is_empty())
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
    let owner = if parents.is_empty() {
        format!("usb{bus}")
    } else {
        format!("{bus}-{}", join_chain(parents))
    };
    Some(port_name(&owner, last))
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
///
/// The generation matters: "USB 3.1 Gen 1" is 5 Gbps, not the 10 Gbps the bare
/// "3.1" suggests, so the Gen suffixes are checked before the version number.
fn bus_name_speed(name: &str) -> f64 {
    let n = name.to_ascii_lowercase();
    if n.contains("usb 4") {
        40000.0
    } else if n.contains("gen 2x2") {
        20000.0
    } else if n.contains("gen 1x2") {
        10000.0
    } else if n.contains("gen 1") {
        5000.0
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology::build_physical_topology;

    // One populated controller carrying:
    //   - an internal 480 Mbps device on root port 1 (USB 2.0-only header —
    //     must NOT be reported as throttled against the 10 Gbps bus)
    //   - a removable 5 Gbps hub on root port 2
    //   - a removable 480 Mbps keyboard on the hub's port 1 (throttled)
    // Plus an empty "USB 4.0 Bus", which gets an index but no root hub.
    const FIXTURE: &str = r#"{
      "SPUSBHostDataType": [
        {
          "_name": "USB 3.1 Bus",
          "USBKeyLocationID": "0x00000000",
          "_items": [
            {
              "_name": "Bluetooth USB Host Controller",
              "USBDeviceKeyVendorID": "0x05ac",
              "USBDeviceKeyProductID": "0x8290",
              "USBDeviceKeyVendorName": "Apple Inc.",
              "USBDeviceKeyLinkSpeed": "480 Mb/s",
              "USBKeyLocationID": "0x00100000",
              "USBKeyHardwareType": "Non-removable"
            },
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
                  "USBKeyHardwareType": "Removable"
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

    fn scan_fixture(json: &str) -> Scan {
        let root: Value = serde_json::from_str(json).unwrap();
        parse_scan(&root, &UsbIds::empty()).unwrap()
    }

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
    fn bus_name_speed_respects_the_generation() {
        let mbps = |name: &str| bus_name_speed(name);
        // The Gen suffix decides, not the version number
        assert!((mbps("USB 3.1 Gen 1 Bus") - 5000.0).abs() < f64::EPSILON);
        assert!((mbps("USB 3.2 Gen 1x2 Bus") - 10000.0).abs() < f64::EPSILON);
        assert!((mbps("USB 3.2 Gen 2x2 Bus") - 20000.0).abs() < f64::EPSILON);
        // Without a Gen suffix the version number is all we have
        assert!((mbps("USB 3.1 Bus") - 10000.0).abs() < f64::EPSILON);
        assert!((mbps("USB 3.0 Bus") - 5000.0).abs() < f64::EPSILON);
        assert!((mbps("USB 2.0 Bus") - 480.0).abs() < f64::EPSILON);
        assert!((mbps("USB 4.0 Bus") - 40000.0).abs() < f64::EPSILON);
        // Unrecognised names claim nothing
        assert!((mbps("Some Controller") - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_scan_builds_flat_devices() {
        let scan = scan_fixture(FIXTURE);

        // Root hub + bluetooth + hub + keyboard; the empty "USB 4.0 Bus"
        // gets no root hub.
        assert_eq!(scan.devices.len(), 4);
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
        assert_eq!(hub.node_key, "1-2");
        assert_eq!(hub.parent_port.as_deref(), Some("usb1-port2"));
        assert_eq!(hub.serial, None); // "Not Provided" is dropped
        assert_eq!(hub.removable.as_deref(), Some("removable"));
        assert!(hub.port_capability_known);

        let kbd = scan
            .devices
            .iter()
            .find(|d| d.product.as_deref() == Some("Gaming Keyboard"))
            .unwrap();
        assert_eq!(kbd.node_key, "1-2.1");
        assert_eq!(kbd.parent_port.as_deref(), Some("1-2-port1"));
        assert_eq!(kbd.vendor_id, 0x1b1c);

        let bt = scan.devices.iter().find(|d| d.vendor_id == 0x05ac).unwrap();
        assert_eq!(bt.node_key, "1-1");
        assert_eq!(bt.removable.as_deref(), Some("fixed"));
        assert!(!bt.port_capability_known);
    }

    #[test]
    fn parse_scan_rejects_unexpected_output() {
        // A renamed/missing data type must be an error, not "no devices"
        let root: Value = serde_json::from_str(r#"{"SPSomethingElse": []}"#).unwrap();
        assert!(parse_scan(&root, &UsbIds::empty()).is_err());
        // An empty machine is still a valid, successful scan
        let root: Value = serde_json::from_str(r#"{"SPUSBHostDataType": []}"#).unwrap();
        assert!(
            parse_scan(&root, &UsbIds::empty())
                .unwrap()
                .devices
                .is_empty()
        );
    }

    #[test]
    fn bus_index_is_stable_when_a_lower_bus_is_empty() {
        // Two controllers, only the higher one populated: it must keep index 2
        // so that plugging something into the empty bus does not renumber it.
        const TWO_BUSES: &str = r#"{
          "SPUSBHostDataType": [
            { "_name": "USB 3.1 Bus", "USBKeyLocationID": "0x00000000" },
            {
              "_name": "USB 2.0 Bus",
              "USBKeyLocationID": "0x02000000",
              "_items": [
                {
                  "_name": "Mouse",
                  "USBDeviceKeyVendorID": "0x046d",
                  "USBDeviceKeyProductID": "0xc52b",
                  "USBDeviceKeyLinkSpeed": "12 Mb/s",
                  "USBKeyLocationID": "0x02100000",
                  "USBKeyHardwareType": "Removable"
                }
              ]
            }
          ]
        }"#;

        let scan = scan_fixture(TWO_BUSES);
        // Only the populated bus gets a root hub, but it keeps index 2
        assert_eq!(scan.devices.len(), 2);
        let root_hub = scan.devices.iter().find(|d| d.is_root_hub()).unwrap();
        assert_eq!(root_hub.bus, 2);
        assert_eq!(root_hub.node_key, "usb2");

        let mouse = scan.devices.iter().find(|d| !d.is_root_hub()).unwrap();
        assert_eq!(mouse.bus, 2);
        assert_eq!(mouse.node_key, "2-1");
        assert_eq!(mouse.parent_port.as_deref(), Some("usb2-port1"));
    }

    #[test]
    fn device_strings_are_stripped_of_control_characters() {
        const EVIL: &str = r#"{
          "SPUSBHostDataType": [
            {
              "_name": "USB 2.0 Bus",
              "USBKeyLocationID": "0x00000000",
              "_items": [
                {
                  "_name": "Evil\u001b]0;pwned\u0007Device",
                  "USBDeviceKeyVendorID": "0x1234",
                  "USBDeviceKeyProductID": "0x5678",
                  "USBDeviceKeyVendorName": "E\u001b[31mvil Corp",
                  "USBDeviceKeyLinkSpeed": "480 Mb/s",
                  "USBKeyLocationID": "0x00100000",
                  "USBKeyHardwareType": "Removable"
                }
              ]
            }
          ]
        }"#;

        let scan = scan_fixture(EVIL);
        let dev = scan.devices.iter().find(|d| !d.is_root_hub()).unwrap();
        assert_eq!(dev.product.as_deref(), Some("Evil]0;pwnedDevice"));
        assert_eq!(dev.manufacturer.as_deref(), Some("E[31mvil Corp"));
        assert!(!dev.display_name().chars().any(char::is_control));
    }

    #[test]
    fn parse_scan_feeds_topology_with_speed_warning() {
        let scan = scan_fixture(FIXTURE);

        let controllers = build_physical_topology(&scan.devices, &scan.peers);
        assert_eq!(controllers.len(), 1);
        assert_eq!(controllers[0].children.len(), 2);

        // Internal 480 Mbps device on a 10 Gbps controller: the port wiring is
        // unknown, so no throttling verdict.
        let bt = &controllers[0].children[0];
        assert_eq!(
            bt.device.product.as_deref(),
            Some("Bluetooth USB Host Controller")
        );
        assert!(!bt.speed_limited);

        let hub = &controllers[0].children[1];
        assert_eq!(hub.device.product.as_deref(), Some("USB3.0 Hub"));
        assert!(!hub.speed_limited);

        // Removable device behind a 5 Gbps hub: verdict stands.
        let kbd = &hub.children[0];
        assert_eq!(kbd.device.product.as_deref(), Some("Gaming Keyboard"));
        assert!(kbd.speed_limited);
        assert!((kbd.port_max_speed - 5000.0).abs() < f64::EPSILON);
    }
}
