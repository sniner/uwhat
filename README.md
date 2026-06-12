# uwhat

A human-friendly USB device lister for Linux. Think of it as a modern
alternative to `lsusb` — designed to show you what's actually plugged into your
machine, not kernel internals.

## Why?

`lsusb` shows raw bus/device numbers and cryptic class codes. `lsusb -t` shows
a tree, but it's a separate view with different information. Figuring out which
physical port a device sits on, or why it's running slow, requires mental
gymnastics across multiple commands.

`uwhat` gives you one clear picture:

```
$ uwhat
Bus 002/001  xHCI Host Controller  20 Gbps
├── Port  3: 2109:8110 VIA Labs, Inc. USB3.0 Hub 5 Gbps
├── Port  4: 11b0:6298 Kingston SNA-DC/U 480 Mbps (of 20 Gbps) [usb-storage]
├── Port  6: 0db0:0076 MSI MYSTIC LIGHT 12 Mbps [usbhid]
└── Port 11: 0489:e10a Foxconn / Hon Hai 12 Mbps [btusb]
Bus 006/005  xHCI Host Controller  10 Gbps
└── Port  2: 05e3:0625 GenesysLogic USB3.2 Hub 10 Gbps
    ├── Port  1: 046d:c52b Logitech USB Receiver 12 Mbps (of 10 Gbps) [usbhid]
    ├── Port  2: 05e3:0625 GenesysLogic USB3.2 Hub 10 Gbps
    │   └── Port  4: 046d:08b6 Logi Webcam C920e 480 Mbps (of 10 Gbps) [uvcvideo, snd-usb-audio]
    └── Port  4: 174c:235c Ugreen Storage Device 480 Mbps (of 10 Gbps) [uas]
Bus 009  xHCI Host Controller  480 Mbps
└── Port  1: 05e3:0608 USB2.0 Hub 480 Mbps
    ├── Port  1: 1b1c:1bc0 Corsair K70 MAX RGB 480 Mbps [usbhid]
    └── Port  2: 2717:5013 MI Mi Wireless Mouse 12 Mbps [usbhid]
```

Things you can see at a glance:

- **Physical port topology** — which device is plugged into which hub and port
- **Speed warnings** — `480 Mbps (of 10 Gbps)` means a USB 2.0 device on a
  USB 3.x port. Maybe a bad cable, maybe the device only supports USB 2.0
- **Drivers** — `[uvcvideo, snd-usb-audio]` tells you which kernel drivers
  claimed the device
- **Companion bus merging** — USB 3.x controllers appear as two Linux buses
  (one for USB 2.0, one for USB 3.x). `uwhat` merges them back into the
  physical reality: `Bus 006/005` is one controller

## How it works

`uwhat` reads directly from Linux sysfs (`/sys/bus/usb/devices/`) — no libusb
or root permissions required. Device and vendor names come from the system's USB
ID database (`/usr/share/hwdata/usb.ids`).

### Companion bus merging

A USB 3.x controller has two signal paths per port: USB 2.0 (the legacy pins)
and USB 3.x (the additional pins). Linux exposes these as separate buses. For
example, buses 005 and 006 might share the same PCI slot — they're the same
physical controller.

`uwhat` merges them back together using the kernel's port peering information
(sysfs `peer` links, available since Linux 3.17): two peered ports are the
USB 2.0 and USB 3.x signal paths of the same physical connector. Buses whose
root ports peer with each other belong to the same controller. The bus label
`Bus 006/005` lists the faster bus first (USB 3.x), then the slower one
(USB 2.0).

When a device negotiates USB 2.0 on a port that is wired for USB 3.x, the
speed annotation `(of 10 Gbps)` appears — a hint that the device could
potentially run faster. Ports without USB 3.x wiring (internal USB 2.0-only
headers, for example) never get this annotation.

## Usage

```
uwhat [OPTIONS] [QUERY]
```

`QUERY` filters devices by case-insensitive substring match against product
name, manufacturer, and kernel driver — `uwhat mouse`, `uwhat logitech`,
`uwhat uvcvideo`. If it looks like a `vendor:product` ID (`uwhat 046d:c52b`),
it filters by ID instead.

| Option | Description |
|--------|-------------|
| `-t`, `--tree` | Show device tree (the default) |
| `-l`, `--list` | Show flat list instead of tree |
| `-j`, `--json` | Output as JSON (always includes full details) |
| `-v` | Show speed, USB version, power, interfaces |
| `-vv` | Show full details (class codes, serial, endpoints) |
| `-d`, `--device VEND:PROD` | Filter by vendor:product ID (hex); either side may be empty, e.g. `046d:c52b`, `046d:` (all Logitech), `:c52b` |
| `-b`, `--bus N` | Filter by bus number |
| `--color WHEN` | Colored output: `auto` (default), `always`, `never`; `auto` honors [`NO_COLOR`](https://no-color.org/) |

### Examples

Default tree view:
```
$ uwhat
```

Flat list with details:
```
$ uwhat -lv
Bus 005 Dev 003: 046d:c52b Logitech USB Receiver [Keyboard]
  Full Speed (12 Mbps), USB 2.00, 98mA
  Interfaces: usbhid (Keyboard), usbhid (Mouse), usbhid (HID)
```

Find a device by name:
```
$ uwhat mouse
Bus 008/007  xHCI Host Controller  10 Gbps
└── Port  1:  0424:7206  Microchip USB7206 Smart Hub  10 Gbps
    └── Port  1:  05e3:0625  GenesysLogic USB3.2 Hub  10 Gbps
        └── Port  1:  2717:5013  MI Mi Wireless Mouse  12 Mbps (of 10 Gbps)  [usbhid]
```

Find a specific device in the tree:
```
$ uwhat -d 046d:c52b
Bus 006/005  xHCI Host Controller  10 Gbps
└── Port  2: 05e3:0625 GenesysLogic USB3.2 Hub 10 Gbps
    └── Port  1: 046d:c52b Logitech USB Receiver 12 Mbps (of 10 Gbps) [usbhid]
```

JSON output for scripting (hierarchical tree by default, flat with `-l`):
```
$ uwhat --json | jq '.[].devices[] | select(.speed_limited) | {name, speed, port_max_speed}'
{
  "name": "Kingston SNA-DC/U",
  "speed": "480 Mbps",
  "port_max_speed": "20 Gbps"
}
```

## Installation

### From source

```
cargo install --path .
```

### Pre-built binary

Download the statically linked binary from the
[releases page](https://github.com/sniner/uwhat/releases).

## Requirements

- **Linux** — `uwhat` reads from sysfs, which is Linux-specific
- `/usr/share/hwdata/usb.ids` — optional, for human-readable vendor/product
  names (provided by `hwdata` or `usbutils` packages on most distributions)

## License

GPLv3 — see [LICENSE](LICENSE).
