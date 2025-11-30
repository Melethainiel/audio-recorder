#!/bin/bash
set -e

echo "Building Audio Recorder release..."

# Build release binary
cargo build --release

# Get architecture
ARCH=$(uname -m)

# Create release directory
mkdir -p release

# Copy binary with architecture suffix
cp target/release/audio-recorder "release/audio-recorder-linux-$ARCH"

# Copy icon
cp icon.png release/audio-recorder.png

echo ""
echo "✓ Release built successfully!"
echo ""
echo "Files created in release/:"
ls -lh release/
echo ""
echo "To create a GitHub release:"
echo "1. Create a new tag: git tag v0.1.0"
echo "2. Push the tag: git push origin v0.1.0"
echo "3. Go to GitHub → Releases → Create a new release"
echo "4. Upload these files from release/ directory"
