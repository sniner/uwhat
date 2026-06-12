#[cfg(not(target_os = "linux"))]
compile_error!("uwhat requires Linux (sysfs)");

mod device;
mod display;
mod json;
mod sysfs;
mod topology;
mod usb_class;
mod usb_ids;

use clap::{ArgAction, Parser};

#[derive(Parser)]
#[command(name = "uwhat", version, about = "Human-friendly USB device lister")]
struct Cli {
    /// Show device tree (default)
    #[arg(short, long, conflicts_with = "list")]
    tree: bool,

    /// Show flat list instead of tree
    #[arg(short, long)]
    list: bool,

    /// Output as JSON (includes full details)
    #[arg(short, long)]
    json: bool,

    /// Increase verbosity (-v, -vv)
    #[arg(short, long, action = ArgAction::Count)]
    verbose: u8,

    /// Filter by vendor:product ID; either side may be empty (e.g. 046d:c52b, 046d:, :c52b)
    #[arg(short, long)]
    device: Option<String>,

    /// Filter by bus number
    #[arg(short, long)]
    bus: Option<u8>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let usb_ids = usb_ids::UsbIds::load();
    let mut devices = sysfs::scan_devices(&usb_ids)?;

    // Parse device filter early so we can use it in both modes
    let device_filter = if let Some(ref filter) = cli.device {
        if let Some(pair) = parse_device_filter(filter) {
            Some(pair)
        } else {
            eprintln!(
                "Invalid device filter '{filter}', expected format: vendor:product \
                 with either side optional (e.g. 046d:c52b, 046d:, :c52b)"
            );
            std::process::exit(1);
        }
    } else {
        None
    };

    let filtering = device_filter.is_some() || cli.bus.is_some();

    if cli.list {
        // Apply filters
        if let Some(id_filter) = device_filter {
            devices.retain(|d| id_filter.matches(d));
        }
        if let Some(bus) = cli.bus {
            devices.retain(|d| d.bus == bus);
        }

        // Hide root hubs in list mode unless filtering
        if cli.bus.is_none() && cli.device.is_none() {
            devices.retain(|d| !d.is_root_hub());
        }

        // Sort by bus, then devpath
        devices.sort_by(|a, b| a.bus.cmp(&b.bus).then(a.devpath.cmp(&b.devpath)));

        if cli.json {
            json::print_list_json(&devices);
        } else {
            display::print_list(&devices, cli.verbose);
        }

        if filtering && devices.is_empty() {
            no_matches();
        }
    } else {
        // Build physical topology (merges companion buses)
        let mut controllers = topology::build_physical_topology(&devices);

        // Apply filters
        if let Some(bus) = cli.bus {
            controllers.retain(|c| c.root_hubs.iter().any(|r| r.bus == bus));
        }
        if let Some(id_filter) = device_filter {
            for ctrl in &mut controllers {
                filter_physical_tree(&mut ctrl.children, id_filter);
            }
            controllers.retain(|c| !c.children.is_empty());
        }

        if cli.json {
            json::print_tree_json(&controllers);
        } else {
            display::print_tree(&controllers, cli.verbose);
        }

        if filtering && controllers.is_empty() {
            no_matches();
        }
    }

    Ok(())
}

/// Report that filters matched nothing and exit with grep-like status 1.
fn no_matches() -> ! {
    eprintln!("uwhat: no matching devices");
    std::process::exit(1);
}

/// A `vendor:product` ID filter where either side may be a wildcard.
#[derive(Clone, Copy)]
struct IdFilter {
    vendor: Option<u16>,
    product: Option<u16>,
}

impl IdFilter {
    fn matches(self, dev: &device::UsbDevice) -> bool {
        self.vendor.is_none_or(|vid| dev.vendor_id == vid)
            && self.product.is_none_or(|pid| dev.product_id == pid)
    }
}

fn parse_device_filter(s: &str) -> Option<IdFilter> {
    let (vendor_part, product_part) = s.split_once(':')?;
    let parse_side = |part: &str| -> Option<Option<u16>> {
        if part.is_empty() {
            Some(None)
        } else {
            u16::from_str_radix(part, 16).ok().map(Some)
        }
    };
    let vendor = parse_side(vendor_part)?;
    let product = parse_side(product_part)?;
    // ":" alone would match everything — treat it as a mistake
    if vendor.is_none() && product.is_none() {
        return None;
    }
    Some(IdFilter { vendor, product })
}

/// Recursively filter physical device tree to only include branches containing matching devices.
fn filter_physical_tree(children: &mut Vec<topology::PhysicalDevice>, filter: IdFilter) {
    children.retain_mut(|pdev| {
        filter_physical_tree(&mut pdev.children, filter);
        filter.matches(pdev.device) || !pdev.children.is_empty()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_filter_full_pair() {
        let f = parse_device_filter("046d:c52b").unwrap();
        assert_eq!(f.vendor, Some(0x046d));
        assert_eq!(f.product, Some(0xc52b));
    }

    #[test]
    fn device_filter_accepts_uppercase() {
        let f = parse_device_filter("046D:C52B").unwrap();
        assert_eq!(f.vendor, Some(0x046d));
        assert_eq!(f.product, Some(0xc52b));
    }

    #[test]
    fn device_filter_vendor_only() {
        let f = parse_device_filter("046d:").unwrap();
        assert_eq!(f.vendor, Some(0x046d));
        assert_eq!(f.product, None);
    }

    #[test]
    fn device_filter_product_only() {
        let f = parse_device_filter(":c52b").unwrap();
        assert_eq!(f.vendor, None);
        assert_eq!(f.product, Some(0xc52b));
    }

    #[test]
    fn device_filter_rejects_invalid() {
        assert!(parse_device_filter(":").is_none());
        assert!(parse_device_filter("").is_none());
        assert!(parse_device_filter("046d").is_none());
        assert!(parse_device_filter("xx:yy").is_none());
        assert!(parse_device_filter("046d:c52b:0").is_none());
        assert!(parse_device_filter("12345:c52b").is_none());
    }
}
