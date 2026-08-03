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

/// Human-readable speed with its USB tier name, e.g. "High Speed (480 Mbps)".
///
/// The tier name is genuine USB terminology and adds information the raw number
/// does not carry, so it stays a lookup. The number in parentheses, though, is
/// formatted straight from the value via [`speed_short`] — no fixed-tier mapping
/// that could misreport USB4's 40 Gbps. Zero means the backend could not
/// determine the speed (macOS omits the link speed for some devices).
pub fn speed_label(speed: f64) -> String {
    if speed <= 0.0 {
        return "unknown speed".to_string();
    }
    let tier = if speed <= 1.5 {
        "Low Speed"
    } else if speed <= 12.0 {
        "Full Speed"
    } else if speed <= 480.0 {
        "High Speed"
    } else if speed <= 5000.0 {
        "SuperSpeed"
    } else if speed <= 20000.0 {
        "SuperSpeed+"
    } else {
        "USB4"
    };
    format!("{tier} ({})", speed_short(speed))
}

/// Compact speed label formatted straight from the value: "480 Mbps", "5 Gbps",
/// "40 Gbps". The link speed already carries the full information, so this only
/// picks Mbps vs Gbps and lets the number speak — no bucketing into fixed tiers
/// that would round, say, 40 Gbps down to 20. Zero means the backend could not
/// determine the speed, see [`speed_label`].
pub fn speed_short(speed: f64) -> String {
    if speed <= 0.0 {
        "unknown".to_string()
    } else if speed < 1000.0 {
        format!("{speed} Mbps")
    } else {
        format!("{} Gbps", speed / 1000.0)
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

    #[test]
    fn speeds_are_formatted_from_the_value_not_bucketed() {
        // Every real USB speed round-trips through the direct formatting …
        assert_eq!(speed_short(12.0), "12 Mbps");
        assert_eq!(speed_short(480.0), "480 Mbps");
        assert_eq!(speed_short(5000.0), "5 Gbps");
        assert_eq!(speed_short(10000.0), "10 Gbps");
        assert_eq!(speed_short(20000.0), "20 Gbps");
        // … including USB4's 40 Gbps, which the old fixed tiers reported as 20
        assert_eq!(speed_short(40000.0), "40 Gbps");
        assert_eq!(speed_label(40000.0), "USB4 (40 Gbps)");
    }
}
