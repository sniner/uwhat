/// A USB device as read from sysfs.
#[allow(dead_code)]
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
                return format!("{} {}", m, p);
            }
            return p.clone();
        }
        if let Some(ref name) = self.product_name {
            if let Some(ref vendor) = self.vendor_name {
                return format!("{} {}", vendor, name);
            }
            return name.clone();
        }
        if let Some(ref vendor) = self.vendor_name {
            return format!("{} {:04x}:{:04x}", vendor, self.vendor_id, self.product_id);
        }
        format!("{:04x}:{:04x}", self.vendor_id, self.product_id)
    }

    /// The last port number from the devpath (for tree display).
    pub fn port_number(&self) -> Option<u16> {
        if self.is_root_hub() {
            return None;
        }
        self.devpath
            .rsplit('.')
            .next()
            .and_then(|s| s.parse().ok())
    }

}
