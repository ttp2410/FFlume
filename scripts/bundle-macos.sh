#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
binary=target/release/fflume
bundle=target/release/FFlume.app
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml)
test -x "$binary"
test -n "$version"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources/Licenses"
install -m 755 "$binary" "$bundle/Contents/MacOS/fflume.new"
mv "$bundle/Contents/MacOS/fflume.new" "$bundle/Contents/MacOS/fflume"
cp LICENSE "$bundle/Contents/Resources/Licenses/FFlume-GPL-3.0.txt"
cp assets/fonts/Geist-OFL.txt assets/fonts/Geist-LICENSE.txt "$bundle/Contents/Resources/Licenses/"
cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>fflume</string>
    <key>CFBundleIdentifier</key><string>org.fflume.desktop</string>
    <key>CFBundleName</key><string>FFlume</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
plutil -lint "$bundle/Contents/Info.plist"
cmp "$binary" "$bundle/Contents/MacOS/fflume"
codesign --force --sign - "$bundle"
codesign --verify --strict "$bundle"
