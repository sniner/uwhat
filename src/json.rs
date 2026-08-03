use std::io::{self, Write};

use serde_json::{Value, json};

use crate::device::{UsbDevice, UsbInterface};
use crate::topology::{PhysicalController, PhysicalDevice};
use crate::usb_class;

/// Print the physical device tree as JSON.
pub fn print_tree_json(controllers: &[PhysicalController]) {
    let arr: Vec<Value> = controllers.iter().map(controller_to_json).collect();
    print_json(&arr);
}

/// Print a flat device list as JSON.
pub fn print_list_json(devices: &[UsbDevice]) {
    let arr: Vec<Value> = devices.iter().map(device_to_json).collect();
    print_json(&arr);
}

/// Write pretty JSON, ignoring write errors (EPIPE) like the text output does.
fn print_json(value: &[Value]) {
    let mut out = io::stdout().lock();
    if let Ok(s) = serde_json::to_string_pretty(value) {
        writeln!(out, "{s}").ok();
    }
}

fn controller_to_json(ctrl: &PhysicalController) -> Value {
    let fastest = &ctrl.root_hubs[0];
    let buses: Vec<u8> = ctrl.root_hubs.iter().map(|r| r.bus).collect();

    json!({
        "pci_slot": ctrl.pci_slot,
        "buses": buses,
        "name": crate::display::controller_name(fastest),
        "speed_mbps": ctrl.max_speed,
        "speed": usb_class::speed_short(ctrl.max_speed),
        "devices": ctrl.children.iter().map(physical_device_to_json).collect::<Vec<_>>(),
    })
}

fn physical_device_to_json(pdev: &PhysicalDevice) -> Value {
    // Common device fields plus the tree-only port information
    let mut obj = device_to_json(pdev.device);
    obj["port"] = json!(pdev.device.port_number());
    obj["port_max_speed_mbps"] = json!(pdev.port_max_speed);
    obj["port_max_speed"] = json!(usb_class::speed_short(pdev.port_max_speed));
    obj["speed_limited"] = json!(pdev.speed_limited);

    if !pdev.children.is_empty() {
        obj["devices"] = json!(
            pdev.children
                .iter()
                .map(physical_device_to_json)
                .collect::<Vec<_>>()
        );
    }

    obj
}

fn device_to_json(dev: &UsbDevice) -> Value {
    // Fields the backend cannot supply are emitted as `null`, not as a zero or
    // an empty list — a consumer must be able to tell "this device has no
    // interfaces" from "this platform does not report interfaces".
    let class = dev
        .device_class
        .as_ref()
        .map(|c| format!("{:02x}:{:02x}:{:02x}", c.class, c.subclass, c.protocol));
    let class_name = dev
        .device_class
        .as_ref()
        .map(|c| usb_class::class_name(c.class));
    let interfaces = dev
        .interfaces
        .as_ref()
        .map(|list| list.iter().map(interface_to_json).collect::<Vec<_>>());
    let drivers = dev.interfaces.as_ref().map(|_| dev.unique_drivers());

    json!({
        "bus": dev.bus,
        "devnum": dev.devnum,
        "devpath": dev.devpath,
        "vendor_id": format!("{:04x}", dev.vendor_id),
        "product_id": format!("{:04x}", dev.product_id),
        "name": dev.display_name(),
        "manufacturer": dev.manufacturer,
        "product": dev.product,
        "serial": dev.serial,
        "speed_mbps": dev.speed,
        "speed": usb_class::speed_short(dev.speed),
        "usb_version": dev.usb_version,
        "class": class,
        "class_name": class_name,
        "max_power": dev.max_power,
        "removable": dev.removable,
        "max_children": dev.max_children,
        "num_interfaces": dev.num_interfaces,
        "interfaces": interfaces,
        "drivers": drivers,
    })
}

fn interface_to_json(iface: &UsbInterface) -> Value {
    json!({
        "number": iface.number,
        "class": format!("{:02x}:{:02x}:{:02x}", iface.class, iface.subclass, iface.protocol),
        "class_name": usb_class::interface_class_name(iface.class, iface.subclass, iface.protocol),
        "num_endpoints": iface.num_endpoints,
        "driver": iface.driver,
    })
}
