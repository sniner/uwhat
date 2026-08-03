use std::collections::HashMap;

/// Result of a platform backend scan: all devices plus the port peer map.
/// This is the neutral contract both the Linux (sysfs) and macOS (`IOKit`)
/// backends produce; everything downstream consumes it unchanged.
///
/// Fields a backend cannot supply are `None`, never a zero or an empty list —
/// so "this device has no interfaces" stays distinguishable from "this
/// platform does not report interfaces".
pub struct Scan {
    pub devices: Vec<UsbDevice>,
    /// Companion port links (both directions), e.g. "usb1-port3" <-> "usb2-port3".
    /// Peered ports are the same physical connector on the USB 2.0 and USB 3.x
    /// side of a controller or hub. Linux-only; always empty on macOS, where
    /// `IOKit` already presents the merged physical topology.
    pub peers: HashMap<String, String>,
}

impl Scan {
    /// Strip control characters from every device-supplied string.
    ///
    /// Descriptor strings come from the device itself; a malicious one could
    /// otherwise inject terminal escape sequences into our output. This runs
    /// once at the platform seam ([`crate::backend::scan`]) so no backend can
    /// forget it — the backends still sanitize on read where they need clean
    /// input for parsing, and [`sanitize`] is idempotent.
    pub fn sanitize_strings(&mut self) {
        for dev in &mut self.devices {
            scrub(&mut dev.manufacturer);
            scrub(&mut dev.product);
            scrub(&mut dev.vendor_name);
            scrub(&mut dev.product_name);
            scrub(&mut dev.serial);
            scrub(&mut dev.max_power);
            scrub(&mut dev.removable);
            scrub(&mut dev.pci_slot);
            for iface in dev.interfaces.iter_mut().flatten() {
                scrub(&mut iface.driver);
            }
        }
    }
}

/// Sanitize an optional field in place; a field that sanitizes to nothing
/// becomes `None`.
fn scrub(field: &mut Option<String>) {
    *field = field.as_deref().map(sanitize).filter(|s| !s.is_empty());
}

/// Strip control characters and surrounding whitespace. Descriptor strings
/// come from the device itself; a malicious one could otherwise inject
/// terminal escape sequences into our output.
pub fn sanitize(content: &str) -> String {
    content
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}

/// The sysfs-style name of a hub port: owner key plus port number, e.g.
/// `("usb1", 3)` -> "usb1-port3", `("1-2", 4)` -> "1-2-port4".
///
/// This is the format both backends must produce and [`port_owner`] parses;
/// keeping the two next to each other is what makes that contract checkable.
pub fn port_name(owner: &str, port: impl std::fmt::Display) -> String {
    format!("{owner}-port{port}")
}

/// The hub instance a port belongs to: "2-3-port1" -> "2-3", "usb1-port6" -> "usb1".
/// Inverse of [`port_name`].
pub fn port_owner(port: &str) -> Option<&str> {
    port.rsplit_once("-port").map(|(owner, _)| owner)
}

/// A USB class triple (class, subclass, protocol) as reported by a descriptor.
pub struct ClassCode {
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
}

/// A USB device as read from a platform backend (Linux sysfs or macOS `IOKit`).
pub struct UsbDevice {
    /// Backend node key: the sysfs directory name on Linux ("5-2.1", "usb1"),
    /// synthesised in the same shape on macOS. Used to link ports to their hub.
    pub node_key: String,
    pub bus: u8,
    pub devnum: u8,
    /// Port topology path, e.g. "2.1" (root hubs have "0")
    pub devpath: String,
    pub vendor_id: u16,
    pub product_id: u16,
    /// Manufacturer string reported by the device itself
    pub manufacturer: Option<String>,
    /// Product string reported by the device itself
    pub product: Option<String>,
    /// From the usb.ids database
    pub vendor_name: Option<String>,
    /// From the usb.ids database
    pub product_name: Option<String>,
    pub serial: Option<String>,
    /// Speed in Mbps (e.g. 480.0, 5000.0); 0.0 means the backend could not
    /// determine it
    pub speed: f64,
    /// USB spec version from the device descriptor, e.g. "2.00". `None` where
    /// the backend does not report it (macOS) — the negotiated speed is then
    /// the only version hint, and inventing "2.00" from it would be a guess
    pub usb_version: Option<String>,
    /// Device-level class triple; `None` where the backend does not report it
    pub device_class: Option<ClassCode>,
    /// e.g. "100mA"
    pub max_power: Option<String>,
    /// `bNumInterfaces` from the device descriptor
    pub num_interfaces: Option<u8>,
    pub removable: Option<String>,
    /// Number of downstream ports (hubs only)
    pub max_children: Option<u8>,
    /// `None` where the backend cannot enumerate interfaces at all (macOS),
    /// as opposed to `Some(vec![])` for a device that genuinely has none
    pub interfaces: Option<Vec<UsbInterface>>,
    /// PCI slot of the root hub this device belongs to (e.g. "0000:77:00.3")
    pub pci_slot: Option<String>,
    /// Name of the hub port this device is attached to (e.g. "usb2-port4" or
    /// "2-3-port1"); None for root hubs
    pub parent_port: Option<String>,
    /// Whether the backend can tell how the parent port is actually wired.
    /// Linux knows it from the sysfs port peer links; macOS does not, so a
    /// USB 2.0-only internal header is indistinguishable from a throttled
    /// `SuperSpeed` port there. Only ports with known capability are reported
    /// as speed-limited.
    pub port_capability_known: bool,
}

pub struct UsbInterface {
    pub number: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub num_endpoints: u8,
    /// Driver basename, e.g. "usbhid", "hub"
    pub driver: Option<String>,
}

impl UsbDevice {
    /// Returns true if this is a root hub (devpath "0").
    pub fn is_root_hub(&self) -> bool {
        self.devpath == "0"
    }

    /// The device's interfaces, empty when the backend does not report any.
    /// Use the [`UsbDevice::interfaces`] field directly to tell "none" from
    /// "not available".
    pub fn interface_list(&self) -> &[UsbInterface] {
        self.interfaces.as_deref().unwrap_or(&[])
    }

    /// Best available display name for this device.
    pub fn display_name(&self) -> String {
        // Prefer the device's own strings, then the usb.ids names
        if let Some(ref p) = self.product {
            return match self.manufacturer {
                Some(ref m) if !product_repeats_manufacturer(m, p) => format!("{m} {p}"),
                _ => p.clone(),
            };
        }
        if let Some(ref name) = self.product_name {
            return match self.vendor_name {
                Some(ref v) if !product_repeats_manufacturer(v, name) => format!("{v} {name}"),
                _ => name.clone(),
            };
        }
        if let Some(ref vendor) = self.vendor_name {
            return format!("{} {:04x}:{:04x}", vendor, self.vendor_id, self.product_id);
        }
        format!("{:04x}:{:04x}", self.vendor_id, self.product_id)
    }

    /// Unique driver names bound to this device (excluding "hub").
    pub fn unique_drivers(&self) -> Vec<&str> {
        let mut unique = Vec::new();
        for iface in self.interface_list() {
            if let Some(ref drv) = iface.driver
                && drv != "hub"
                && !unique.contains(&drv.as_str())
            {
                unique.push(drv.as_str());
            }
        }
        unique
    }

    /// The last port number from the devpath (for tree display).
    pub fn port_number(&self) -> Option<u16> {
        if self.is_root_hub() {
            return None;
        }
        self.devpath.rsplit('.').next().and_then(|s| s.parse().ok())
    }

    /// The devpath as numeric segments, for sorting ("1.10" after "1.2").
    pub fn devpath_segments(&self) -> Vec<u16> {
        self.devpath
            .split('.')
            .filter_map(|s| s.parse().ok())
            .collect()
    }

    /// Case-insensitive substring match against names and bound drivers.
    /// `query` must already be lowercase.
    pub fn matches_text(&self, query: &str) -> bool {
        let names = [
            self.manufacturer.as_deref(),
            self.product.as_deref(),
            self.vendor_name.as_deref(),
            self.product_name.as_deref(),
        ];
        names
            .iter()
            .flatten()
            .any(|s| s.to_lowercase().contains(query))
            || self
                .interface_list()
                .iter()
                .filter_map(|i| i.driver.as_deref())
                .any(|d| d.to_lowercase().contains(query))
    }
}

/// Whether `product` already carries the manufacturer's name, so that
/// prefixing it would read "SMSL SMSL USB AUDIO".
///
/// Compares the manufacturer's leading word against the product's words. A
/// plain substring test does not do: it misses "Apple Inc." in "Apple Internal
/// Keyboard" (the legal suffix is not in the product) and would match any
/// fragment mid-word.
fn product_repeats_manufacturer(manufacturer: &str, product: &str) -> bool {
    let Some(brand) = words(manufacturer).next() else {
        return false;
    };
    // Single letters match far too much to be evidence of anything
    if brand.chars().count() < 2 {
        return false;
    }
    words(product).any(|w| w.eq_ignore_ascii_case(brand))
}

fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_device() -> UsbDevice {
        UsbDevice {
            node_key: "1-2".to_string(),
            bus: 1,
            devnum: 2,
            devpath: "2".to_string(),
            vendor_id: 0x046d,
            product_id: 0xc52b,
            manufacturer: Some("Logitech".to_string()),
            product: Some("USB Receiver".to_string()),
            vendor_name: Some("Logitech, Inc.".to_string()),
            product_name: Some("Unifying Receiver".to_string()),
            serial: None,
            speed: 12.0,
            usb_version: Some("2.00".to_string()),
            device_class: Some(ClassCode {
                class: 0,
                subclass: 0,
                protocol: 0,
            }),
            max_power: None,
            num_interfaces: Some(1),
            removable: None,
            max_children: None,
            interfaces: Some(vec![UsbInterface {
                number: 0,
                class: 0x03,
                subclass: 0x01,
                protocol: 0x02,
                num_endpoints: 1,
                driver: Some("usbhid".to_string()),
            }]),
            pci_slot: None,
            parent_port: Some("usb1-port2".to_string()),
            port_capability_known: true,
        }
    }

    #[test]
    fn matches_text_searches_all_name_fields() {
        let dev = test_device();
        assert!(dev.matches_text("logitech"));
        assert!(dev.matches_text("receiver"));
        assert!(dev.matches_text("unifying"));
    }

    #[test]
    fn matches_text_searches_drivers() {
        let dev = test_device();
        assert!(dev.matches_text("usbhid"));
    }

    #[test]
    fn matches_text_rejects_non_matches() {
        let dev = test_device();
        assert!(!dev.matches_text("webcam"));
    }

    #[test]
    fn matches_text_survives_missing_interface_data() {
        let mut dev = test_device();
        dev.interfaces = None;
        assert!(dev.matches_text("logitech"));
        assert!(!dev.matches_text("usbhid"));
    }

    #[test]
    fn devpath_segments_sort_numerically() {
        let mut a = test_device();
        a.devpath = "1.10".to_string();
        let mut b = test_device();
        b.devpath = "1.2".to_string();
        assert!(a.devpath_segments() > b.devpath_segments());
    }

    #[test]
    fn display_name_keeps_a_distinct_manufacturer() {
        let dev = test_device();
        assert_eq!(dev.display_name(), "Logitech USB Receiver");
    }

    #[test]
    fn display_name_drops_a_manufacturer_the_product_repeats() {
        let mut dev = test_device();
        // Same word, different case
        dev.manufacturer = Some("Corsair".to_string());
        dev.product = Some("CORSAIR K70 RGB Keyboard".to_string());
        assert_eq!(dev.display_name(), "CORSAIR K70 RGB Keyboard");

        // Legal suffix in the manufacturer only — a substring test would miss this
        dev.manufacturer = Some("Apple Inc.".to_string());
        dev.product = Some("Apple Internal Keyboard".to_string());
        assert_eq!(dev.display_name(), "Apple Internal Keyboard");

        // Word boundaries are respected: "VIA" is not part of "USB3.0 Hub"
        dev.manufacturer = Some("VIA Labs, Inc.".to_string());
        dev.product = Some("USB3.0 Hub".to_string());
        assert_eq!(dev.display_name(), "VIA Labs, Inc. USB3.0 Hub");
    }

    #[test]
    fn display_name_falls_back_to_usb_ids_names() {
        let mut dev = test_device();
        dev.manufacturer = None;
        dev.product = None;
        assert_eq!(dev.display_name(), "Logitech, Inc. Unifying Receiver");

        dev.vendor_name = None;
        dev.product_name = None;
        assert_eq!(dev.display_name(), "046d:c52b");
    }

    #[test]
    fn sanitize_strips_escape_sequences() {
        assert_eq!(
            sanitize("Evil\x1b]0;pwned\x07Device\n"),
            "Evil]0;pwnedDevice"
        );
        assert_eq!(sanitize("  USB Receiver \n"), "USB Receiver");
        assert_eq!(sanitize("\x1b[2J\x1b[H"), "[2J[H");
    }

    #[test]
    fn sanitize_strings_scrubs_every_device_supplied_field() {
        let mut dev = test_device();
        dev.manufacturer = Some("Log\x1b[31mitech".to_string());
        dev.product = Some("\x1b]0;pwned\x07Receiver".to_string());
        dev.serial = Some("\x1b\x1b".to_string()); // sanitizes to nothing
        dev.interfaces.as_mut().unwrap()[0].driver = Some("usb\x1bhid".to_string());

        let mut scan = Scan {
            devices: vec![dev],
            peers: HashMap::new(),
        };
        scan.sanitize_strings();

        let dev = &scan.devices[0];
        assert_eq!(dev.manufacturer.as_deref(), Some("Log[31mitech"));
        assert_eq!(dev.product.as_deref(), Some("]0;pwnedReceiver"));
        assert_eq!(dev.serial, None);
        assert_eq!(dev.interface_list()[0].driver.as_deref(), Some("usbhid"));
    }

    #[test]
    fn port_name_and_port_owner_round_trip() {
        assert_eq!(port_name("usb1", 3), "usb1-port3");
        assert_eq!(port_name("1-2", "4"), "1-2-port4");
        assert_eq!(port_owner(&port_name("1-2.3", 4)).unwrap(), "1-2.3");
        assert_eq!(port_owner(&port_name("usb11", 1)).unwrap(), "usb11");
        assert_eq!(port_owner("usb1"), None);
    }
}
