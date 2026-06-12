# Changelog

Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.2.0] - 2026-06-12

### Breaking changes

- **Exit status**: when `--device`, `--bus`, or a search query matches no devices, `uwhat` now
  prints a note to stderr and exits with status 1 (grep semantics). Scripts that relied on
  exit 0 with empty output must check the exit code instead
- **`--tree`/`--list`**: combining both flags is now an error; previously `--list` silently won

### Added

- **Search**: a free positional argument filters by case-insensitive substring match against
  product name, manufacturer, and kernel driver — `uwhat mouse`, `uwhat logitech`. Queries
  shaped like a `vendor:product` ID (`uwhat 046d:c52b`) filter by ID instead
- **`--device`**: either side of the ID may now be empty — `046d:` matches all devices of a
  vendor, `:c52b` all devices with that product ID
- **`--color`**: new option `auto|always|never`; `auto` (the default) honors the
  [`NO_COLOR`](https://no-color.org/) convention
- **`--completions`**: print shell completion scripts for bash, zsh, fish, elvish, or powershell
- **Tree header**: `-v` shows the controller's PCI slot

### Changed

- **Companion bus merging** now uses the kernel's port peering information (sysfs `peer`
  links, Linux 3.17+) instead of assuming that port numbers align between the USB 2.0 and
  USB 3.x bus of a controller
- **JSON output**: `pci_slot` may now be `null` for controllers without a PCI slot
- **Tree header** shows the controller product name ("xHCI Host Controller") instead of the
  full manufacturer string with kernel version and driver; JSON controller `name` follows
- **List mode**: root hubs are hidden unless a device filter or search query matches them;
  a bare `--bus` no longer reveals them
- **`--device`**: an invalid filter value now exits with status 2 (clap convention)
  instead of 1
- **Security**: control characters are stripped from device-supplied descriptor strings, so a
  malicious device can no longer inject terminal escape sequences into the output

### Fixed

- **Speed hints**: devices on USB 2.0-only ports (internal headers, hub ports without USB 3.x
  wiring) no longer show a bogus `(of 20 Gbps)` annotation
- **Tree view**: devices could be silently hidden when two different devices sat on the same
  port number of companion buses; physical ports are now identified exactly via peer links
- **Tree view**: buses whose root hub reports no PCI slot no longer disappear from the output
- **List mode**: devices are sorted by numeric port path, so port 1.10 follows 1.2
- **Robustness**: a malformed usb.ids line or a single unreadable sysfs entry no longer
  aborts the program; `uwhat --json | head` no longer risks a broken-pipe panic

## [0.1.1] - 2026

### Added

- **`--json`**: machine-readable output for both tree and list mode, always with full details

### Fixed

- **Display**: spacing between speed, warning, and driver fields in color mode

## [0.1.0] - 2026

Initial release.
