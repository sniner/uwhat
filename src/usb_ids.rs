use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// USB ID database mapping vendor/product IDs to human-readable names.
pub struct UsbIds {
    vendors: HashMap<u16, String>,
    products: HashMap<(u16, u16), String>,
}

/// Where a system-installed `usb.ids` may live — the Linux FHS locations.
/// macOS ships no such database and none is searched for: names there come from
/// the devices themselves, and a device that reports none shows its bare ID
/// rather than making the output depend on an optional third-party package.
const USB_IDS_PATHS: &[&str] = &[
    "/usr/share/hwdata/usb.ids",
    "/usr/share/misc/usb.ids",
    "/var/lib/usbutils/usb.ids",
];

impl UsbIds {
    /// An empty database (no names). Used when no `usb.ids` is available and in
    /// tests that need a hermetic lookup.
    pub fn empty() -> Self {
        Self {
            vendors: HashMap::new(),
            products: HashMap::new(),
        }
    }

    /// Load the USB ID database from the system. Returns an empty database on failure.
    pub fn load() -> Self {
        for path in USB_IDS_PATHS {
            if Path::new(path).exists()
                && let Ok(content) = fs::read_to_string(path)
            {
                return Self::parse(&content);
            }
        }
        Self::empty()
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
                if let Some(vid) = current_vendor
                    && let Some((pid, name)) = parse_id_line(line.trim_start())
                {
                    products.insert((vid, pid), name);
                }
            } else if !line.starts_with('\t') {
                // Vendor line: VVVV  vendor_name
                if let Some((vid, name)) = parse_id_line(line) {
                    vendors.insert(vid, name);
                    current_vendor = Some(vid);
                } else {
                    current_vendor = None;
                }
            }
        }

        Self { vendors, products }
    }

    pub fn vendor_name(&self, vendor_id: u16) -> Option<&str> {
        self.vendors
            .get(&vendor_id)
            .map(std::string::String::as_str)
    }

    pub fn product_name(&self, vendor_id: u16, product_id: u16) -> Option<&str> {
        self.products
            .get(&(vendor_id, product_id))
            .map(std::string::String::as_str)
    }
}

/// Parse an "XXXX  name" line into ID and name. Uses `get()` so lines with
/// multi-byte characters in unexpected positions are skipped, not panicked on.
fn parse_id_line(line: &str) -> Option<(u16, String)> {
    let id = u16::from_str_radix(line.get(..4)?, 16).ok()?;
    let name = line.get(4..)?.trim();
    if name.is_empty() {
        return None;
    }
    Some((id, name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "\
# usb.ids fixture
046d  Logitech, Inc.
\tc52b  Unifying Receiver
\t\t01  some interface detail
05e3  Genesys Logic, Inc.
\t0608  Hub
äöü malformed multibyte line
C 03  HID
\t01  ignored after class section
";

    #[test]
    fn parses_vendors_and_products() {
        let ids = UsbIds::parse(FIXTURE);
        assert_eq!(ids.vendor_name(0x046d), Some("Logitech, Inc."));
        assert_eq!(ids.product_name(0x046d, 0xc52b), Some("Unifying Receiver"));
        assert_eq!(ids.vendor_name(0x05e3), Some("Genesys Logic, Inc."));
        assert_eq!(ids.product_name(0x05e3, 0x0608), Some("Hub"));
    }

    #[test]
    fn stops_at_class_section() {
        let ids = UsbIds::parse(FIXTURE);
        assert_eq!(ids.product_name(0x05e3, 0x0001), None);
    }

    #[test]
    fn survives_multibyte_garbage() {
        // Must not panic on non-ASCII bytes at the slice boundary
        let ids = UsbIds::parse("é234  bad\n0123  good\n");
        assert_eq!(ids.vendor_name(0x0123), Some("good"));
    }
}
