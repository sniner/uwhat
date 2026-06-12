#[cfg(not(target_os = "linux"))]
compile_error!("uwhat requires Linux (sysfs)");

mod device;
mod display;
mod json;
mod sysfs;
mod topology;
mod usb_class;
mod usb_ids;

use std::io::IsTerminal;

use clap::{ArgAction, Parser, ValueEnum};

#[derive(Parser)]
#[command(name = "uwhat", version, about = "Human-friendly USB device lister")]
struct Cli {
    /// Show only matching devices: case-insensitive name/driver search,
    /// or a vendor:product ID like 046d:c52b
    query: Option<String>,

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

    /// When to use colored output
    #[arg(long, value_enum, default_value_t = ColorMode::Auto)]
    color: ColorMode,
}

#[derive(Clone, Copy, ValueEnum)]
// Doc comments double as clap help text, where backticks would show up literally.
#[allow(clippy::doc_markdown)]
enum ColorMode {
    /// Color when stdout is a terminal and NO_COLOR is not set
    Auto,
    Always,
    Never,
}

impl ColorMode {
    fn use_color(self) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => {
                let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
                !no_color && std::io::stdout().is_terminal()
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let usb_ids = usb_ids::UsbIds::load();
    let sysfs::Scan { mut devices, peers } = sysfs::scan_devices(&usb_ids)?;

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

    // A free-form query filters by name unless it looks like a vendor:product ID
    let query = cli.query.as_deref().map(Query::parse);

    let device_pred = |d: &device::UsbDevice| {
        device_filter.is_none_or(|f| f.matches(d)) && query.as_ref().is_none_or(|q| q.matches(d))
    };
    let device_filtering = device_filter.is_some() || query.is_some();
    let filtering = device_filtering || cli.bus.is_some();

    if cli.list {
        // Apply filters
        devices.retain(|d| device_pred(d));
        if let Some(bus) = cli.bus {
            devices.retain(|d| d.bus == bus);
        }

        // Hide root hubs in list mode unless filtering
        if !filtering {
            devices.retain(|d| !d.is_root_hub());
        }

        // Sort by bus, then devpath (numerically, so port 10 comes after port 2)
        devices.sort_by(|a, b| {
            a.bus
                .cmp(&b.bus)
                .then_with(|| a.devpath_segments().cmp(&b.devpath_segments()))
        });

        if cli.json {
            json::print_list_json(&devices);
        } else {
            display::print_list(&devices, cli.verbose, cli.color.use_color());
        }

        if filtering && devices.is_empty() {
            no_matches();
        }
    } else {
        // Build physical topology (merges companion buses)
        let mut controllers = topology::build_physical_topology(&devices, &peers);

        // Apply filters
        if let Some(bus) = cli.bus {
            controllers.retain(|c| c.root_hubs.iter().any(|r| r.bus == bus));
        }
        if device_filtering {
            for ctrl in &mut controllers {
                filter_physical_tree(&mut ctrl.children, &device_pred);
            }
            controllers.retain(|c| !c.children.is_empty());
        }

        if cli.json {
            json::print_tree_json(&controllers);
        } else {
            display::print_tree(&controllers, cli.verbose, cli.color.use_color());
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

/// A free-form query: a `vendor:product` ID if it parses as one, otherwise a name search.
enum Query {
    Id(IdFilter),
    Name(String),
}

impl Query {
    fn parse(s: &str) -> Self {
        parse_device_filter(s).map_or_else(|| Self::Name(s.to_lowercase()), Self::Id)
    }

    fn matches(&self, dev: &device::UsbDevice) -> bool {
        match self {
            Self::Id(filter) => filter.matches(dev),
            Self::Name(query) => dev.matches_text(query),
        }
    }
}

/// Recursively filter physical device tree to only include branches containing matching devices.
fn filter_physical_tree(
    children: &mut Vec<topology::PhysicalDevice>,
    pred: &impl Fn(&device::UsbDevice) -> bool,
) {
    children.retain_mut(|pdev| {
        filter_physical_tree(&mut pdev.children, pred);
        pred(pdev.device) || !pdev.children.is_empty()
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
    fn query_parses_id_syntax_as_id_filter() {
        assert!(matches!(Query::parse("046d:c52b"), Query::Id(_)));
        assert!(matches!(Query::parse("046d:"), Query::Id(_)));
        assert!(matches!(Query::parse(":c52b"), Query::Id(_)));
    }

    #[test]
    fn query_falls_back_to_lowercased_name_search() {
        let Query::Name(q) = Query::parse("Mouse") else {
            panic!("expected name query");
        };
        assert_eq!(q, "mouse");
        // A colon alone is not a valid ID filter, so it is a name search
        assert!(matches!(Query::parse(":"), Query::Name(_)));
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
