use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// USB ID database mapping vendor/product IDs to human-readable names.
pub struct UsbIds {
    vendors: HashMap<u16, String>,
    products: HashMap<(u16, u16), String>,
}

const USB_IDS_PATHS: &[&str] = &[
    "/usr/share/hwdata/usb.ids",
    "/usr/share/misc/usb.ids",
    "/var/lib/usbutils/usb.ids",
];

impl UsbIds {
    /// Load the USB ID database from the system. Returns an empty database on failure.
    pub fn load() -> Self {
        for path in USB_IDS_PATHS {
            if Path::new(path).exists()
                && let Ok(content) = fs::read_to_string(path)
            {
                return Self::parse(&content);
            }
        }
        Self {
            vendors: HashMap::new(),
            products: HashMap::new(),
        }
    }

    fn parse(content: &str) -> Self {
        let mut vendors = HashMap::new();
        let mut products = HashMap::new();
        let mut current_vendor: Option<u16> = None;

        for line in content.lines() {
            // Skip comments and empty lines
            if line.starts_with('#') || line.is_empty() {
                continue;
            }

            // Sections like "C 00  ..." (class definitions) — stop parsing
            if line.starts_with("C ") {
                break;
            }

            if line.starts_with('\t') && !line.starts_with("\t\t") {
                // Product line: \tPPPP  product_name
                if let Some(vid) = current_vendor {
                    let trimmed = line.trim_start();
                    if trimmed.len() >= 6
                        && let Ok(pid) = u16::from_str_radix(&trimmed[..4], 16)
                    {
                        let name = trimmed[4..].trim().to_string();
                        if !name.is_empty() {
                            products.insert((vid, pid), name);
                        }
                    }
                }
            } else if !line.starts_with('\t') && line.len() >= 6 {
                // Vendor line: VVVV  vendor_name
                if let Ok(vid) = u16::from_str_radix(&line[..4], 16) {
                    let name = line[4..].trim().to_string();
                    if !name.is_empty() {
                        vendors.insert(vid, name);
                        current_vendor = Some(vid);
                    }
                } else {
                    current_vendor = None;
                }
            }
        }

        Self { vendors, products }
    }

    pub fn vendor_name(&self, vendor_id: u16) -> Option<&str> {
        self.vendors.get(&vendor_id).map(|s| s.as_str())
    }

    pub fn product_name(&self, vendor_id: u16, product_id: u16) -> Option<&str> {
        self.products
            .get(&(vendor_id, product_id))
            .map(|s| s.as_str())
    }
}
