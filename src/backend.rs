//! Platform backend seam.
//!
//! [`scan`] is the single entry point the rest of the program calls to obtain
//! the neutral [`Scan`]. Which backend runs is decided at compile time by the
//! target OS: Linux reads sysfs, macOS reads the `IOKit` USB tree via
//! `system_profiler`. Everything downstream (topology, display, JSON, filters)
//! is identical for both.
//!
//! What the seam guarantees on top of whatever a backend returns:
//! [`Scan::sanitize_strings`] runs here, so no backend can ship device-supplied
//! strings with terminal escape sequences still in them.

use crate::device::Scan;
use crate::usb_ids::UsbIds;

/// Scan the system's USB devices using the platform-appropriate backend.
pub fn scan(usb_ids: &UsbIds) -> Result<Scan, Box<dyn std::error::Error>> {
    let mut scan = platform_scan(usb_ids)?;
    scan.sanitize_strings();
    Ok(scan)
}

#[cfg(target_os = "linux")]
fn platform_scan(usb_ids: &UsbIds) -> Result<Scan, Box<dyn std::error::Error>> {
    crate::sysfs::scan_devices(usb_ids)
}

#[cfg(target_os = "macos")]
fn platform_scan(usb_ids: &UsbIds) -> Result<Scan, Box<dyn std::error::Error>> {
    crate::macos::scan_devices(usb_ids)
}
