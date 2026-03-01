# Release Notes — openbci v0.0.1

**Release date:** March 1, 2026  
**Crates.io:** [crates.io/crates/openbci](https://crates.io/crates/openbci)  
**Docs:** [docs.rs/openbci](https://docs.rs/openbci)  
**License:** MIT

---

## 🎉 Initial Release

`openbci` is a pure-Rust driver for OpenBCI EEG/EMG boards — no C/C++ runtime, no BrainFlow dependency. It communicates directly with hardware over USB serial, Bluetooth LE, WiFi Shield, and UDP.

---

## ✨ Features

### Supported Boards

| Board | Channels | Interface |
|---|---|---|
| **Cyton** | 8 EEG | USB serial (FTDI dongle) |
| **Cyton + Daisy** | 16 EEG | USB serial (FTDI dongle) |
| **Cyton WiFi** | 8 EEG | OpenBCI WiFi Shield → TCP |
| **Cyton Daisy WiFi** | 16 EEG | OpenBCI WiFi Shield → TCP |
| **Ganglion** | 4 EEG | Bluetooth LE (`ble` feature) |
| **Ganglion WiFi** | 4 EEG | OpenBCI WiFi Shield → TCP |
| **Galea** | 24 EEG+EMG | UDP |

### Electrode Placement & Montage
- Full support for the **10-05**, **10-10**, and **10-20** standard electrode systems (83, 176, and 334 sites respectively)
- Named position constants (`positions::FP1`, `positions::CZ`, etc.) with 3-D MNI head-surface coordinates sourced from MNE-Python
- `ElectrodeLayout` for mapping channel indices to electrode labels, signal types (EEG/EMG), and positions
- Alias resolution (e.g. `"T3"` → `"T7"`)
- Pre-built layouts: `cyton_motor()`, `cyton_daisy_standard()`, `ganglion_default()`

### ADS1299 Channel Configuration
- Per-channel control of gain (1×, 2×, 4×, 6×, 8×, 12×, 24×), input type, bias network inclusion, and SRB2 reference via `ChannelConfig`
- `apply_channel_config()` (single channel) and `apply_all_channel_configs()` (all at once)
- `reset_to_defaults()` to restore factory ADS1299 settings

### Streaming API
- `start_stream()` returns a `StreamHandle` supporting:
  - **Blocking iterator** — `for sample in stream { … }`
  - **Non-blocking polling** — `stream.try_recv()`
  - **Explicit stop** — `stream.stop()`
  - **Auto-stop on drop**
- Background reader thread with automatic Cyton/Daisy packet pairing for 16-channel merged samples at ~125 Hz

### Sample Data
Every board emits `Sample` values containing:
- `eeg: Vec<f64>` — µV per channel
- `accel: Option<[f64; 3]>` — accelerometer in g (X, Y, Z)
- `analog: Option<[f64; 3]>` — raw ADC values (Cyton analog mode)
- `resistance: Option<Vec<f64>>` — impedance in Ω (Ganglion)
- `timestamp: f64` — seconds since UNIX epoch (host clock)
- `sample_num: u8` — rolling counter (gaps indicate dropped packets)

### Galea Board
Decodes all 24 heterogeneous channels plus biometric sensors:
- Upper-face EMG (8 ch, gain 4×), EEG 10-20 (10 ch, gain 12×), auxiliary EMG (4 ch)
- EDA (skin conductance), PPG (red/IR), temperature, battery level
- IMU: accelerometer, gyroscope, magnetometer

### WiFi Shield Support
- Auto-discovery via SSDP (pass an empty IP string)
- Configurable HTTP timeout and local TCP listener port

---

## 📦 Installation

```toml
[dependencies]
openbci = "0.0.1"

# Without Ganglion BLE support (smaller dependency tree, no tokio/btleplug):
openbci = { version = "0.0.1", default-features = false }
```

---

## ⚙️ Cargo Features

| Feature | Default | Description |
|---|---|---|
| `ble` | ✅ | Ganglion BLE support via `btleplug` + `tokio` |

---

## 🖥️ Platform Support

| Platform | Serial | BLE |
|---|---|---|
| **Linux** | `/dev/ttyUSB*`, `/dev/ttyACM*` | BlueZ (`libdbus-dev` required) |
| **macOS** | `/dev/tty.usbserial-*` | CoreBluetooth (no extra setup) |
| **Windows** | `COM3`, etc. | WinRT (Windows 10 v1703+) |

**Linux tip:** Set FTDI latency timer to 1 ms for best performance:
```sh
echo 1 | sudo tee /sys/bus/usb-serial/devices/ttyUSB0/latency_timer
```

---

## 📖 Examples

```sh
cargo run --example cyton_stream -- /dev/ttyUSB0
cargo run --example cyton_daisy_stream -- /dev/ttyUSB0
cargo run --example ganglion_ble
cargo run --example wifi_stream
cargo run --example galea_stream -- 192.168.1.200
```

---

## 📚 References

This library implements protocol and signal-processing details described in the following works:
- OpenBCI hardware documentation (Cyton, Daisy, Ganglion, Galea, WiFi Shield)
- Texas Instruments ADS1299 datasheet (SBAS499C)
- Jasper (1958) — 10-20 electrode system
- American EEG Society (1994) — 10-10 electrode nomenclature
- Oostenveld & Praamstra (2001) — 10-05 electrode system
- Gramfort et al. (2013, 2014) — MNE-Python (electrode positions)

Full BibTeX citations are available in [`REFERENCES.bib`](REFERENCES.bib).

---

## Citing

```bibtex
@software{kosmyna2026openbci,
  author    = {Kosmyna, Nataliya},
  title     = {{openbci}: A Pure-{R}ust Driver for {OpenBCI} {EEG}/{EMG} Boards},
  year      = {2026},
  version   = {0.0.1},
  url       = {https://github.com/nataliyakosmyna/openbci},
  note      = {Crates.io: \url{https://crates.io/crates/openbci}},
  license   = {MIT},
}
```

---

*Issues and pull requests are welcome. Please include hardware type and firmware version when reporting bugs.*
