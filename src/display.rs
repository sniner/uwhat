use std::io::{self, IsTerminal, Write};

use owo_colors::OwoColorize;

use crate::device::UsbDevice;
use crate::topology::{PhysicalController, PhysicalDevice};
use crate::usb_class;

/// Print devices in list format.
pub fn print_list(devices: &[UsbDevice], verbose: u8) {
    let use_color = io::stdout().is_terminal();
    let mut out = io::stdout().lock();

    for dev in devices {
        print_device_line(&mut out, dev, verbose, use_color);
    }
}

fn print_device_line(out: &mut impl Write, dev: &UsbDevice, verbose: u8, color: bool) {
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

    if color {
        write!(
            out,
            "Bus {:03} Dev {:03}: {} {}",
            dev.bus,
            dev.devnum,
            id.cyan(),
            name.bold(),
        )
        .ok();
    } else {
        write!(out, "Bus {:03} Dev {:03}: {} {}", dev.bus, dev.devnum, id, name).ok();
    }

    if let Some(label) = class_label {
        if color {
            write!(out, " [{}]", label.yellow()).ok();
        } else {
            write!(out, " [{}]", label).ok();
        }
    }

    writeln!(out).ok();

    if verbose >= 1 {
        print_verbose_1(out, dev, color);
    }
    if verbose >= 2 {
        print_verbose_2(out, dev, color);
    }
}

fn print_verbose_1(out: &mut impl Write, dev: &UsbDevice, color: bool) {
    let speed = usb_class::speed_label(dev.speed);
    if color {
        writeln!(
            out,
            "  {}, USB {}, {}",
            speed.green(),
            dev.usb_version,
            dev.max_power.as_deref().unwrap_or("?"),
        )
        .ok();
    } else {
        writeln!(
            out,
            "  {}, USB {}, {}",
            speed,
            dev.usb_version,
            dev.max_power.as_deref().unwrap_or("?"),
        )
        .ok();
    }

    if !dev.interfaces.is_empty() {
        let ifaces: Vec<String> = dev
            .interfaces
            .iter()
            .map(|i| {
                let class = usb_class::interface_class_name(i.class, i.subclass, i.protocol);
                match &i.driver {
                    Some(drv) if color => format!("{} ({})", drv.blue(), class),
                    Some(drv) => format!("{} ({})", drv, class),
                    None if color => format!("- ({})", class),
                    None => format!("- ({})", class),
                }
            })
            .collect();
        writeln!(out, "  Interfaces: {}", ifaces.join(", ")).ok();
    }
}

fn print_verbose_2(out: &mut impl Write, dev: &UsbDevice, _color: bool) {
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
        writeln!(out, "  Serial: {}", serial).ok();
    }
    if let Some(ref removable) = dev.removable {
        writeln!(out, "  Removable: {}", removable).ok();
    }
    if let Some(children) = dev.max_children {
        writeln!(out, "  Hub ports: {}", children).ok();
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
        return Some(usb_class::interface_class_name(i.class, i.subclass, i.protocol));
    }
    None
}

// --- Tree display ---

/// Print the physical device tree (companion buses merged).
pub fn print_tree(controllers: &[PhysicalController], verbose: u8) {
    let use_color = io::stdout().is_terminal();
    let mut out = io::stdout().lock();

    for ctrl in controllers {
        // Controller header: show PCI slot, name from fastest root hub, max speed
        let fastest = &ctrl.root_hubs[0]; // sorted by speed desc
        let name = fastest.display_name();
        let speed = usb_class::speed_short(ctrl.max_speed);
        let buses: Vec<String> = ctrl.root_hubs.iter().map(|r| format!("{:03}", r.bus)).collect();
        let bus_label = buses.join("/");

        if use_color {
            writeln!(out, "Bus {}  {}  {}", bus_label, name.bold(), speed.green()).ok();
        } else {
            writeln!(out, "Bus {}  {}  {}", bus_label, name, speed).ok();
        }

        if !ctrl.children.is_empty() {
            print_physical_children(&mut out, &ctrl.children, "", verbose, use_color);
        }
    }
}

fn unique_drivers(dev: &UsbDevice) -> Vec<&str> {
    let mut unique = Vec::new();
    for iface in &dev.interfaces {
        if let Some(ref drv) = iface.driver {
            if drv != "hub" && !unique.contains(&drv.as_str()) {
                unique.push(drv.as_str());
            }
        }
    }
    unique
}

fn print_physical_children(
    out: &mut impl Write,
    children: &[PhysicalDevice],
    prefix: &str,
    verbose: u8,
    color: bool,
) {
    let count = children.len();
    for (i, pdev) in children.iter().enumerate() {
        let is_last = i == count - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last {
            format!("{}    ", prefix)
        } else {
            format!("{}│   ", prefix)
        };

        let dev = pdev.device;
        let port = dev.port_number().unwrap_or(0);
        let id = format!("{:04x}:{:04x}", dev.vendor_id, dev.product_id);
        let name = dev.display_name();
        let speed = usb_class::speed_short(dev.speed);

        let drivers = unique_drivers(dev);
        let driver_str = if drivers.is_empty() {
            String::new()
        } else {
            format!(" [{}]", drivers.join(", "))
        };

        // Speed warning for devices running below port capability
        let speed_warning = if pdev.speed_limited {
            format!(" (of {})", usb_class::speed_short(pdev.port_max_speed))
        } else {
            String::new()
        };

        if color {
            write!(
                out,
                "{}{}Port {:2}: {} {} {}",
                prefix,
                connector,
                port,
                id.cyan(),
                name.bold(),
                speed.green(),
            )
            .ok();
            if !speed_warning.is_empty() {
                write!(out, " {}", speed_warning.yellow()).ok();
            }
            if !driver_str.is_empty() {
                write!(out, " {}", driver_str.blue()).ok();
            }
        } else {
            write!(
                out,
                "{}{}Port {:2}: {} {} {}{}{}",
                prefix, connector, port, id, name, speed, speed_warning, driver_str,
            )
            .ok();
        }

        if verbose >= 1 {
            write!(out, ", USB {}", dev.usb_version).ok();
            if let Some(ref power) = dev.max_power {
                write!(out, ", {}", power).ok();
            }
        }

        writeln!(out).ok();

        if verbose >= 2 {
            let detail_prefix = if !pdev.children.is_empty() {
                format!("{}│   ", child_prefix)
            } else {
                format!("{}    ", child_prefix)
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
            print_physical_children(out, &pdev.children, &child_prefix, verbose, color);
        }
    }
}
