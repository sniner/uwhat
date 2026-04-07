use serde_json::{json, Value};

use crate::device::{UsbDevice, UsbInterface};
use crate::topology::{PhysicalController, PhysicalDevice};
use crate::usb_class;

/// Print the physical device tree as JSON.
pub fn print_tree_json(controllers: &[PhysicalController]) {
    let arr: Vec<Value> = controllers.iter().map(controller_to_json).collect();
    println!("{}", serde_json::to_string_pretty(&arr).unwrap());
}

/// Print a flat device list as JSON.
pub fn print_list_json(devices: &[UsbDevice]) {
    let arr: Vec<Value> = devices.iter().map(device_to_json).collect();
    println!("{}", serde_json::to_string_pretty(&arr).unwrap());
}

fn controller_to_json(ctrl: &PhysicalController) -> Value {
    let fastest = &ctrl.root_hubs[0];
    let buses: Vec<u8> = ctrl.root_hubs.iter().map(|r| r.bus).collect();

    json!({
        "pci_slot": ctrl.pci_slot,
        "buses": buses,
        "name": fastest.display_name(),
        "speed_mbps": ctrl.max_speed,
        "speed": usb_class::speed_short(ctrl.max_speed),
        "devices": ctrl.children.iter().map(physical_device_to_json).collect::<Vec<_>>(),
    })
}

fn physical_device_to_json(pdev: &PhysicalDevice) -> Value {
    let dev = pdev.device;
    let drivers = dev.unique_drivers();

    let mut obj = json!({
        "port": dev.port_number(),
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
        "port_max_speed_mbps": pdev.port_max_speed,
        "port_max_speed": usb_class::speed_short(pdev.port_max_speed),
        "speed_limited": pdev.speed_limited,
        "usb_version": dev.usb_version,
        "class": format!("{:02x}:{:02x}:{:02x}", dev.device_class, dev.device_subclass, dev.device_protocol),
        "class_name": usb_class::class_name(dev.device_class),
        "max_power": dev.max_power,
        "removable": dev.removable,
        "max_children": dev.max_children,
        "interfaces": dev.interfaces.iter().map(interface_to_json).collect::<Vec<_>>(),
        "drivers": drivers,
    });

    if !pdev.children.is_empty() {
        obj["devices"] = json!(pdev.children.iter().map(physical_device_to_json).collect::<Vec<_>>());
    }

    obj
}

fn device_to_json(dev: &UsbDevice) -> Value {
    let drivers = dev.unique_drivers();

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
        "class": format!("{:02x}:{:02x}:{:02x}", dev.device_class, dev.device_subclass, dev.device_protocol),
        "class_name": usb_class::class_name(dev.device_class),
        "max_power": dev.max_power,
        "removable": dev.removable,
        "max_children": dev.max_children,
        "interfaces": dev.interfaces.iter().map(interface_to_json).collect::<Vec<_>>(),
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

