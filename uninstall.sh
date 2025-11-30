#!/bin/bash
set -e

echo "Uninstalling Audio Recorder..."

sudo rm -f /usr/local/bin/audio-recorder
sudo rm -f /usr/share/pixmaps/audio-recorder.png
sudo rm -f /usr/share/applications/audio-recorder.desktop

echo "✓ Audio Recorder uninstalled successfully!"
