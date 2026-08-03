# Changelog

Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- **macOS support**: `uwhat` now runs on macOS in addition to Linux. It reads the USB tree from
  `system_profiler SPUSBHostDataType`; the default tree view, name/ID filtering and `--json` all
  work as on Linux. Per-interface driver annotations (`[usbhid]`) and the full `-vv` class-code
  detail are Linux-only, since macOS does not expose that data through this source
- **Release binaries** for macOS (universal, Apple Silicon and Intel) and for `aarch64` Linux,
  alongside the existing `x86_64` Linux build
- **`num_interfaces`** in `--json`, the interface count from the device descriptor

### Changed

- **Device names** no longer repeat the manufacturer when the product name already contains it:
  `SMSL SMSL USB AUDIO` is now `SMSL USB AUDIO`, `MI Mi Wireless Mouse` is `Mi Wireless Mouse`.
  A manufacturer that is genuinely absent from the product name is still prefixed as before
- **Speed warnings** (`(of 10 Gbps)`) are now only shown where the port's wiring is actually
  known. On Linux that is every port, via the sysfs port peer links. macOS does not report it,
  so the hint appears for removable ports but is withheld for built-in devices, which sit on
  USB 2.0-only internal headers often enough that the warning would mostly be a false alarm
- **Unknown link speeds** are reported as `unknown` instead of being rounded down to
  `1.5 Mbps`. Affects macOS, where `system_profiler` omits the link speed for some devices
- **`--json` reports unavailable data as `null`**, not as a zero or an empty list. `class`,
  `class_name`, `usb_version`, `num_interfaces`, `interfaces` and `drivers` are `null` on macOS,
  where the underlying source does not expose them — previously they were indistinguishable
  from a device that genuinely has no interfaces or no class code. Unchanged on Linux
- **`-v`/`-vv` omit fields the platform does not report** instead of printing `?` or zeros

### Fixed

- **Device names** from a device's own descriptors are now stripped of terminal control
  characters on macOS as well, not just on Linux. A malicious USB device could otherwise have
  injected escape sequences into `uwhat`'s output
- **`--bus` on macOS** now refers to a stable bus number. Previously the synthetic numbering
  covered only buses that had devices on them, so plugging a device into a so-far empty bus
  renumbered the others
- **Unexpected `system_profiler` output** is now reported as an error instead of being shown as
  an empty device list, which was indistinguishable from a machine with nothing plugged in
- **Devices whose parent hub could not be read** are shown under their controller instead of
  disappearing from the tree along with everything below them; they were only visible in
  `--list` before
- **`USB 3.1 Gen 1` and `USB 3.2 Gen 1x2` buses on macOS** are no longer reported at the speed of
  the fastest generation of that version (10 Gbps for both), which also produced spurious speed
  warnings for devices on them

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
