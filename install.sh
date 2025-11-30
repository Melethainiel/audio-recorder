#!/bin/bash
set -e

REPO="Melethainiel/audio-recorder"
VERSION="latest"

echo "Installing Audio Recorder..."

# Check if running as root
if [ "$EUID" -eq 0 ]; then
    echo "Error: Do not run this script as root (without sudo)"
    echo "The script will ask for sudo when needed"
    exit 1
fi

# Detect architecture
ARCH=$(uname -m)
case $ARCH in
    x86_64)
        BINARY_NAME="audio-recorder-linux-x86_64"
        ;;
    aarch64)
        BINARY_NAME="audio-recorder-linux-aarch64"
        ;;
    *)
        echo "Error: Unsupported architecture: $ARCH"
        echo "Supported: x86_64, aarch64"
        exit 1
        ;;
esac

# Create temporary directory
TMP_DIR=$(mktemp -d)
trap "rm -rf $TMP_DIR" EXIT

cd "$TMP_DIR"

# Download binary from GitHub releases
echo "Downloading Audio Recorder..."
if command -v curl &> /dev/null; then
    curl -L "https://github.com/$REPO/releases/latest/download/$BINARY_NAME" -o audio-recorder
elif command -v wget &> /dev/null; then
    wget "https://github.com/$REPO/releases/latest/download/$BINARY_NAME" -O audio-recorder
else
    echo "Error: Neither curl nor wget found. Please install one of them."
    exit 1
fi

# Download icon
echo "Downloading icon..."
if command -v curl &> /dev/null; then
    curl -L "https://github.com/$REPO/releases/latest/download/audio-recorder.png" -o audio-recorder.png
else
    wget "https://github.com/$REPO/releases/latest/download/audio-recorder.png" -O audio-recorder.png
fi

# Make binary executable
chmod +x audio-recorder

# Install binary
echo "Installing to /usr/local/bin..."
sudo install -Dm755 audio-recorder /usr/local/bin/audio-recorder

# Install icon
sudo install -Dm644 audio-recorder.png /usr/share/pixmaps/audio-recorder.png

# Create desktop entry
echo "Creating desktop entry..."
sudo tee /usr/share/applications/audio-recorder.desktop > /dev/null << EOF
[Desktop Entry]
Name=Audio Recorder
Comment=Record audio from microphone and system audio
Exec=/usr/local/bin/audio-recorder
Icon=audio-recorder
Terminal=false
Type=Application
Categories=AudioVideo;Audio;Recorder;
Keywords=audio;record;microphone;
StartupWMClass=audio-recorder
EOF

echo ""
echo "✓ Audio Recorder installed successfully!"
echo ""
echo "You can now:"
echo "  - Launch it from your application menu"
echo "  - Run 'audio-recorder' in terminal"
echo "  - Click the system tray icon to show/hide the window"
