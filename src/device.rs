/// A USB device as read from sysfs.
pub struct UsbDevice {
    /// sysfs directory name, e.g. "5-2.1" or "usb1"
    pub sysfs_name: String,
    pub bus: u8,
    pub devnum: u8,
    /// Port topology path, e.g. "2.1" (root hubs have "0")
    pub devpath: String,
    pub vendor_id: u16,
    pub product_id: u16,
    /// From sysfs `manufacturer` attribute
    pub manufacturer: Option<String>,
    /// From sysfs `product` attribute
    pub product: Option<String>,
    /// From usb.ids database
    pub vendor_name: Option<String>,
    /// From usb.ids database
    pub product_name: Option<String>,
    pub serial: Option<String>,
    /// Speed in Mbps (e.g. 480.0, 5000.0)
    pub speed: f64,
    /// USB spec version, e.g. "2.00"
    pub usb_version: String,
    pub device_class: u8,
    pub device_subclass: u8,
    pub device_protocol: u8,
    /// e.g. "100mA"
    pub max_power: Option<String>,
    pub num_interfaces: u8,
    pub removable: Option<String>,
    /// Number of downstream ports (hubs only)
    pub max_children: Option<u8>,
    pub interfaces: Vec<UsbInterface>,
    /// PCI slot of the root hub this device belongs to (e.g. "0000:77:00.3")
    pub pci_slot: Option<String>,
    /// sysfs name of the hub port this device is attached to
    /// (e.g. "usb2-port4" or "2-3-port1"); None for root hubs
    pub parent_port: Option<String>,
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

    /// Best available display name for this device.
    pub fn display_name(&self) -> String {
        // Prefer sysfs product, then usb.ids product name
        if let Some(ref p) = self.product {
            if let Some(ref m) = self.manufacturer {
                return format!("{m} {p}");
            }
            return p.clone();
        }
        if let Some(ref name) = self.product_name {
            if let Some(ref vendor) = self.vendor_name {
                return format!("{vendor} {name}");
            }
            return name.clone();
        }
        if let Some(ref vendor) = self.vendor_name {
            return format!("{} {:04x}:{:04x}", vendor, self.vendor_id, self.product_id);
        }
        format!("{:04x}:{:04x}", self.vendor_id, self.product_id)
    }

    /// Unique driver names bound to this device (excluding "hub").
    pub fn unique_drivers(&self) -> Vec<&str> {
        let mut unique = Vec::new();
        for iface in &self.interfaces {
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
                .interfaces
                .iter()
                .filter_map(|i| i.driver.as_deref())
                .any(|d| d.to_lowercase().contains(query))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_device() -> UsbDevice {
        UsbDevice {
            sysfs_name: "1-2".to_string(),
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
            usb_version: "2.00".to_string(),
            device_class: 0,
            device_subclass: 0,
            device_protocol: 0,
            max_power: None,
            num_interfaces: 1,
            removable: None,
            max_children: None,
            interfaces: vec![UsbInterface {
                number: 0,
                class: 0x03,
                subclass: 0x01,
                protocol: 0x02,
                num_endpoints: 1,
                driver: Some("usbhid".to_string()),
            }],
            pci_slot: None,
            parent_port: Some("usb1-port2".to_string()),
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
    fn devpath_segments_sort_numerically() {
        let mut a = test_device();
        a.devpath = "1.10".to_string();
        let mut b = test_device();
        b.devpath = "1.2".to_string();
        assert!(a.devpath_segments() > b.devpath_segments());
    }
}
