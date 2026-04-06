#[cfg(not(target_os = "linux"))]
compile_error!("uwhat requires Linux (sysfs)");

mod device;
mod display;
mod sysfs;
mod topology;
mod usb_class;
mod usb_ids;

use clap::{ArgAction, Parser};

#[derive(Parser)]
#[command(name = "uwhat", version, about = "Human-friendly USB device lister")]
struct Cli {
    /// Show device tree (default)
    #[arg(short, long)]
    tree: bool,

    /// Show flat list instead of tree
    #[arg(short, long)]
    list: bool,

    /// Increase verbosity (-v, -vv)
    #[arg(short, long, action = ArgAction::Count)]
    verbose: u8,

    /// Filter by vendor:product ID (e.g. 046d:c52b)
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
                "Invalid device filter '{}', expected format: vendor:product (e.g. 046d:c52b)",
                filter
            );
            std::process::exit(1);
        }
    } else {
        None
    };

    if cli.list {
        // Apply filters
        if let Some((vid, pid)) = device_filter {
            devices.retain(|d| d.vendor_id == vid && d.product_id == pid);
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

        display::print_list(&devices, cli.verbose);
    } else {
        // Build physical topology (merges companion buses)
        let mut controllers = topology::build_physical_topology(&devices);

        // Apply filters
        if let Some(bus) = cli.bus {
            controllers.retain(|c| c.root_hubs.iter().any(|r| r.bus == bus));
        }
        if let Some((vid, pid)) = device_filter {
            for ctrl in &mut controllers {
                filter_physical_tree(&mut ctrl.children, vid, pid);
            }
            controllers.retain(|c| !c.children.is_empty());
        }

        display::print_tree(&controllers, cli.verbose);
    }

    Ok(())
}

fn parse_device_filter(s: &str) -> Option<(u16, u16)> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let vid = u16::from_str_radix(parts[0], 16).ok()?;
    let pid = u16::from_str_radix(parts[1], 16).ok()?;
    Some((vid, pid))
}

/// Recursively filter physical device tree to only include branches containing the given device.
fn filter_physical_tree(children: &mut Vec<topology::PhysicalDevice>, vid: u16, pid: u16) {
    children.retain_mut(|pdev| {
        filter_physical_tree(&mut pdev.children, vid, pid);
        let matches = pdev.device.vendor_id == vid && pdev.device.product_id == pid;
        matches || !pdev.children.is_empty()
    });
}
