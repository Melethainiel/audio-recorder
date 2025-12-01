# Audio Recorder

Simple audio recorder for Linux with GTK4 interface. Record from microphone and system audio (loopback).

## Features

- Record microphone audio
- Record system audio (loopback)
- Mix microphone and system audio
- Adjustable microphone gain
- Minimal popup interface
- System tray integration
- Persistent settings
- **Custom save directory** - Choose where to save recordings
- **N8N webhook integration** - Automatically upload recordings to N8N endpoint
- **Flexible storage options** - Keep files locally, upload only, or both

## Installation

### Quick Install (Recommended)

Download and run the installer:

```bash
curl -sSL https://raw.githubusercontent.com/YOUR_USERNAME/audio-recorder/master/install.sh | bash
```

Or download and inspect first:

```bash
wget https://raw.githubusercontent.com/YOUR_USERNAME/audio-recorder/master/install.sh
chmod +x install.sh
./install.sh
```

### Uninstall

```bash
./uninstall.sh
```

## Usage

1. Launch from application menu or run `audio-recorder`
2. Click the system tray icon to show/hide the window
3. Click ⚙ to configure audio sources and settings:
   - **Microphone**: Select your input device
   - **System Audio (Loopback)**: Capture system audio
   - **Microphone Gain**: Adjust input volume (-20dB to +20dB)
   - **Save Directory**: Choose where to save recordings (defaults to current directory)
   - **N8N Upload**: 
     - Enable/disable automatic upload to N8N webhook
     - Configure endpoint URL
     - Choose whether to keep files locally after upload
4. Click ⏺ to start recording
5. Click ⏹ to stop and save

### Recording Storage

By default, recordings are saved as `.ogg` files in the current directory. You can customize this in Settings:

- **Custom Directory**: Use the file picker to choose a specific folder
- **N8N Upload**: Automatically POST recordings to an N8N webhook endpoint as multipart/form-data
- **Hybrid Mode**: Enable both local storage and N8N upload

## Building from Source

### Requirements

- Rust (1.70+)
- GTK4
- PulseAudio/PipeWire

### Build

```bash
cargo build --release
```

### Create Release Package

```bash
./build-release.sh
```

This creates binaries in `release/` directory ready for GitHub releases.

## Development

- `cargo run` - Run in debug mode
- `cargo build --release` - Build optimized binary
- `./install.sh` - Install locally (for developers, builds from source)

## License

MIT

## Requirements

- Linux with PulseAudio or PipeWire
- GTK4
- System tray support (for tray icon)
