use std::io::{self, Write};

use owo_colors::{OwoColorize, Style};

use crate::device::UsbDevice;
use crate::topology::{PhysicalController, PhysicalDevice};
use crate::usb_class;

/// Output styling. With color disabled every style is a no-op and tree
/// fields are separated by two spaces instead of one — a deliberate
/// readability aid when color cannot set the fields apart.
struct Theme {
    id: Style,
    name: Style,
    speed: Style,
    driver: Style,
    warn: Style,
    label: Style,
    sep: &'static str,
}

impl Theme {
    fn new(color: bool) -> Self {
        if color {
            Self {
                id: Style::new().cyan(),
                name: Style::new().bold(),
                speed: Style::new().green(),
                driver: Style::new().blue(),
                warn: Style::new().yellow(),
                label: Style::new().yellow(),
                sep: " ",
            }
        } else {
            Self {
                id: Style::new(),
                name: Style::new(),
                speed: Style::new(),
                driver: Style::new(),
                warn: Style::new(),
                label: Style::new(),
                sep: "  ",
            }
        }
    }
}

/// Print devices in list format.
pub fn print_list(devices: &[UsbDevice], verbose: u8, use_color: bool) {
    let theme = Theme::new(use_color);
    let mut out = io::stdout().lock();

    for dev in devices {
        print_device_line(&mut out, dev, verbose, &theme);
    }
}

fn print_device_line(out: &mut impl Write, dev: &UsbDevice, verbose: u8, theme: &Theme) {
    let id = format!("{:04x}:{:04x}", dev.vendor_id, dev.product_id);
    let name = dev.display_name();

    // Device class label (only if meaningful at device level)
    let class_label = if dev.device_class != 0x00 && dev.device_class != 0x09 {
        Some(usb_class::class_name(dev.device_class))
    } else if dev.device_class == 0x09 {
        Some("Hub")
    } else {
        effective_class_label(dev)
    };

    write!(
        out,
        "Bus {:03} Dev {:03}: {} {}",
        dev.bus,
        dev.devnum,
        id.style(theme.id),
        name.style(theme.name),
    )
    .ok();

    if let Some(label) = class_label {
        write!(out, " [{}]", label.style(theme.label)).ok();
    }

    writeln!(out).ok();

    if verbose >= 1 {
        print_verbose_1(out, dev, theme);
    }
    if verbose >= 2 {
        print_verbose_2(out, dev);
    }
}

fn print_verbose_1(out: &mut impl Write, dev: &UsbDevice, theme: &Theme) {
    let speed = usb_class::speed_label(dev.speed);
    writeln!(
        out,
        "  {}, USB {}, {}",
        speed.style(theme.speed),
        dev.usb_version,
        dev.max_power.as_deref().unwrap_or("?"),
    )
    .ok();

    if !dev.interfaces.is_empty() {
        let ifaces: Vec<String> = dev
            .interfaces
            .iter()
            .map(|i| {
                let class = usb_class::interface_class_name(i.class, i.subclass, i.protocol);
                match &i.driver {
                    Some(drv) => format!("{} ({class})", drv.style(theme.driver)),
                    None => format!("- ({class})"),
                }
            })
            .collect();
        writeln!(out, "  Interfaces: {}", ifaces.join(", ")).ok();
    }
}

fn print_verbose_2(out: &mut impl Write, dev: &UsbDevice) {
    writeln!(
        out,
        "  Class: {:02x}:{:02x}:{:02x} ({}), {} interface(s)",
        dev.device_class,
        dev.device_subclass,
        dev.device_protocol,
        usb_class::class_name(dev.device_class),
        dev.num_interfaces,
    )
    .ok();
    if let Some(ref serial) = dev.serial {
        writeln!(out, "  Serial: {serial}").ok();
    }
    if let Some(ref removable) = dev.removable {
        writeln!(out, "  Removable: {removable}").ok();
    }
    if let Some(children) = dev.max_children {
        writeln!(out, "  Hub ports: {children}").ok();
    }
    for iface in &dev.interfaces {
        writeln!(
            out,
            "  Interface {}: class {:02x}:{:02x}:{:02x}, {} endpoint(s), driver: {}",
            iface.number,
            iface.class,
            iface.subclass,
            iface.protocol,
            iface.num_endpoints,
            iface.driver.as_deref().unwrap_or("(none)"),
        )
        .ok();
    }
}

/// Derive a class label from interfaces when device class is 0x00.
fn effective_class_label(dev: &UsbDevice) -> Option<&'static str> {
    if dev.interfaces.is_empty() {
        return None;
    }
    // If all interfaces share the same class, use that
    let first = dev.interfaces[0].class;
    if dev.interfaces.iter().all(|i| i.class == first) && first != 0x00 {
        // Use the most specific label from the first interface
        let i = &dev.interfaces[0];
        return Some(usb_class::interface_class_name(
            i.class, i.subclass, i.protocol,
        ));
    }
    None
}

// --- Tree display ---

/// Print the physical device tree (companion buses merged).
pub fn print_tree(controllers: &[PhysicalController], verbose: u8, use_color: bool) {
    let theme = Theme::new(use_color);
    let mut out = io::stdout().lock();

    for ctrl in controllers {
        // Controller header: buses, name from fastest root hub, max speed
        let fastest = &ctrl.root_hubs[0]; // sorted by speed desc
        let name = controller_name(fastest);
        let speed = usb_class::speed_short(ctrl.max_speed);
        let buses: Vec<String> = ctrl
            .root_hubs
            .iter()
            .map(|r| format!("{:03}", r.bus))
            .collect();
        let bus_label = buses.join("/");

        write!(
            out,
            "Bus {}  {}  {}",
            bus_label,
            name.style(theme.name),
            speed.style(theme.speed)
        )
        .ok();
        if verbose >= 1
            && let Some(ref slot) = ctrl.pci_slot
        {
            write!(out, "  [{slot}]").ok();
        }
        writeln!(out).ok();

        if !ctrl.children.is_empty() {
            print_physical_children(&mut out, &ctrl.children, "", verbose, &theme);
        }
    }
}

/// Header name for a controller. Root hubs carry the kernel version and
/// driver in their manufacturer string ("Linux 6.x xhci-hcd"); the product
/// alone ("xHCI Host Controller") is what a human wants to see.
pub fn controller_name(root_hub: &UsbDevice) -> String {
    root_hub
        .product
        .clone()
        .unwrap_or_else(|| root_hub.display_name())
}

fn print_physical_children(
    out: &mut impl Write,
    children: &[PhysicalDevice],
    prefix: &str,
    verbose: u8,
    theme: &Theme,
) {
    let sep = theme.sep;
    let count = children.len();
    for (i, pdev) in children.iter().enumerate() {
        let is_last = i == count - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last {
            format!("{prefix}    ")
        } else {
            format!("{prefix}│   ")
        };

        let dev = pdev.device;
        let port = dev.port_number().unwrap_or(0);
        let id = format!("{:04x}:{:04x}", dev.vendor_id, dev.product_id);
        let name = dev.display_name();
        let speed = usb_class::speed_short(dev.speed);

        write!(
            out,
            "{prefix}{connector}Port {port:2}:{sep}{}{sep}{}{sep}{}",
            id.style(theme.id),
            name.style(theme.name),
            speed.style(theme.speed),
        )
        .ok();

        // Speed warning for devices running below port capability
        if pdev.speed_limited {
            let warning = format!("(of {})", usb_class::speed_short(pdev.port_max_speed));
            write!(out, " {}", warning.style(theme.warn)).ok();
        }

        let drivers = dev.unique_drivers();
        if !drivers.is_empty() {
            let driver_str = format!("[{}]", drivers.join(", "));
            write!(out, "{sep}{}", driver_str.style(theme.driver)).ok();
        }

        if verbose >= 1 {
            write!(out, ", USB {}", dev.usb_version).ok();
            if let Some(ref power) = dev.max_power {
                write!(out, ", {power}").ok();
            }
        }

        writeln!(out).ok();

        if verbose >= 2 {
            let detail_prefix = if pdev.children.is_empty() {
                format!("{child_prefix}    ")
            } else {
                format!("{child_prefix}│   ")
            };
            for iface in &dev.interfaces {
                let class =
                    usb_class::interface_class_name(iface.class, iface.subclass, iface.protocol);
                writeln!(
                    out,
                    "{}intf {}: {} (driver: {})",
                    detail_prefix,
                    iface.number,
                    class,
                    iface.driver.as_deref().unwrap_or("none"),
                )
                .ok();
            }
        }

        if !pdev.children.is_empty() {
            print_physical_children(out, &pdev.children, &child_prefix, verbose, theme);
        }
    }
}
