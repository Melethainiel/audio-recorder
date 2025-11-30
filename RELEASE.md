# Release Process

## How to create a new release

### 1. Build the release binaries

```bash
./build-release.sh
```

This creates:
- `release/audio-recorder-linux-x86_64` - Binary for x86_64
- `release/audio-recorder.png` - Icon

### 2. Create a git tag

```bash
git tag v0.1.0
git push origin v0.1.0
```

### 3. Create GitHub Release

1. Go to https://github.com/YOUR_USERNAME/audio-recorder/releases
2. Click "Draft a new release"
3. Select the tag you just created (v0.1.0)
4. Set release title: "v0.1.0"
5. Write release notes (features, fixes, etc.)
6. Upload the files from `release/`:
   - `audio-recorder-linux-x86_64`
   - `audio-recorder.png`
7. Click "Publish release"

### 4. Users can now install with:

```bash
curl -sSL https://raw.githubusercontent.com/YOUR_USERNAME/audio-recorder/master/install.sh | bash
```

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
