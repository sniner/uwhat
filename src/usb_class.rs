/// Human-readable name for a USB device class code.
pub fn class_name(class: u8) -> &'static str {
    match class {
        0x00 => "Per Interface",
        0x01 => "Audio",
        0x02 => "Communications",
        0x03 => "HID",
        0x05 => "Physical",
        0x06 => "Image",
        0x07 => "Printer",
        0x08 => "Mass Storage",
        0x09 => "Hub",
        0x0a => "CDC-Data",
        0x0b => "Smart Card",
        0x0d => "Content Security",
        0x0e => "Video",
        0x0f => "Healthcare",
        0x10 => "Audio/Video",
        0xdc => "Diagnostic",
        0xe0 => "Wireless",
        0xef => "Miscellaneous",
        0xfe => "Application Specific",
        0xff => "Vendor Specific",
        _ => "Unknown",
    }
}

/// Human-readable name for a USB interface class code.
pub fn interface_class_name(class: u8, subclass: u8, protocol: u8) -> &'static str {
    match (class, subclass, protocol) {
        (0x03, 0x01, 0x01) => "Keyboard",
        (0x03, 0x01, 0x02) => "Mouse",
        (0x03, _, _) => "HID",
        (0x08, 0x06, _) => "Mass Storage (SCSI)",
        (0x08, 0x04, _) => "Mass Storage (UFI)",
        (0x08, _, _) => "Mass Storage",
        (0x0e, 0x01, _) => "Video Control",
        (0x0e, 0x02, _) => "Video Streaming",
        (0x0e, _, _) => "Video",
        (0x01, 0x01, _) => "Audio Control",
        (0x01, 0x02, _) => "Audio Streaming",
        (0x01, 0x03, _) => "MIDI Streaming",
        (0x01, _, _) => "Audio",
        (0x02, 0x02, _) => "Modem",
        (0x02, 0x06, _) => "Ethernet",
        (0x02, 0x0d, _) => "Network (NCM)",
        (0x02, _, _) => "Communications",
        (0x0a, _, _) => "CDC-Data",
        (0x07, _, _) => "Printer",
        (0x09, _, _) => "Hub",
        (0xe0, 0x01, 0x01) => "Bluetooth",
        (0xe0, _, _) => "Wireless",
        (0xfe, 0x01, _) => "DFU",
        (0xfe, _, _) => "Application Specific",
        (0xff, _, _) => "Vendor Specific",
        _ => class_name(class),
    }
}

/// Human-readable speed label. A speed of zero means the backend could not
/// determine it (macOS omits the link speed for some devices) — that is
/// reported as unknown rather than as the slowest USB speed.
pub fn speed_label(speed: f64) -> &'static str {
    if speed <= 0.0 {
        "unknown speed"
    } else if speed <= 1.5 {
        "Low Speed (1.5 Mbps)"
    } else if speed <= 12.0 {
        "Full Speed (12 Mbps)"
    } else if speed <= 480.0 {
        "High Speed (480 Mbps)"
    } else if speed <= 5000.0 {
        "SuperSpeed (5 Gbps)"
    } else if speed <= 10000.0 {
        "SuperSpeed+ (10 Gbps)"
    } else {
        "SuperSpeed+ (20 Gbps)"
    }
}

/// Short speed label for compact display. Zero means unknown, see [`speed_label`].
pub fn speed_short(speed: f64) -> &'static str {
    if speed <= 0.0 {
        "unknown"
    } else if speed <= 1.5 {
        "1.5 Mbps"
    } else if speed <= 12.0 {
        "12 Mbps"
    } else if speed <= 480.0 {
        "480 Mbps"
    } else if speed <= 5000.0 {
        "5 Gbps"
    } else if speed <= 10000.0 {
        "10 Gbps"
    } else {
        "20 Gbps"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_speed_is_not_reported_as_low_speed() {
        assert_eq!(speed_short(0.0), "unknown");
        assert_eq!(speed_label(0.0), "unknown speed");
        // The slowest real USB speed still reports as such
        assert_eq!(speed_short(1.5), "1.5 Mbps");
        assert_eq!(speed_label(1.5), "Low Speed (1.5 Mbps)");
    }
}
