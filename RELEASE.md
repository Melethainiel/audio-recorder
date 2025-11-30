# Release Process

## How to create a new release (Automated)

### 1. Commit your changes

```bash
git add .
git commit -m "Your changes"
git push
```

### 2. Create and push a version tag

```bash
git tag v0.1.0
git push origin v0.1.0
```

**That's it!** 🎉 

GitHub Actions will automatically:
- Build the release binary
- Create a GitHub release
- Upload the binary and icon
- Add installation instructions

### 3. Check the release

Go to https://github.com/YOUR_USERNAME/audio-recorder/releases

The release will be created automatically in a few minutes.

## Manual build (optional)

If you want to build locally:

```bash
./build-release.sh
```

This creates binaries in `release/` directory.

## Version numbering

Use semantic versioning: `MAJOR.MINOR.PATCH`

- MAJOR: Breaking changes
- MINOR: New features
- PATCH: Bug fixes

Examples:
- v0.1.0 - Initial release
- v0.1.1 - Bug fix
- v0.2.0 - New feature
- v1.0.0 - First stable release

## What happens when you push a tag?

1. GitHub Actions detects the new tag
2. Sets up Ubuntu with GTK4, PulseAudio dependencies
3. Installs Rust toolchain
4. Builds the release binary (`cargo build --release`)
5. Creates a new GitHub Release
6. Uploads `audio-recorder-linux-x86_64` binary
7. Uploads `audio-recorder.png` icon
8. Adds installation instructions to the release notes

## Users installation

After the release is published, users can install with:

```bash
curl -sSL https://raw.githubusercontent.com/YOUR_USERNAME/audio-recorder/master/install.sh | bash
```
