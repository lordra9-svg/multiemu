#!/usr/bin/env bash
# Builds a Linux AppImage.
#
# Downloads linuxdeploy and appimagetool into target/appimage-tools on first
# use. AppImages are not bit-reproducible (the tools embed timestamps), so the
# tar.gz from scripts/package.py remains the reproducible artifact.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
tools="$root/target/appimage-tools"
out=${1:-"$root/dist"}
arch=${ARCH:-x86_64}
version=$(grep -m1 '^version' "$root/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')

mkdir -p "$tools" "$out"

fetch() {
  local url=$1 dest=$2
  if [ ! -x "$dest" ]; then
    echo "fetching $(basename "$dest")"
    curl -fsSL "$url" -o "$dest"
    chmod +x "$dest"
  fi
}

fetch "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-${arch}.AppImage" \
  "$tools/linuxdeploy"
fetch "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${arch}.AppImage" \
  "$tools/appimagetool"

export APPIMAGE_EXTRACT_AND_RUN=1

cd "$root"
cargo build --release --locked -p unko-cli -p unko-app

appdir="$root/target/AppDir"
rm -rf "$appdir"
install -Dm755 target/release/unko-app "$appdir/usr/bin/unko-app"
install -Dm755 target/release/unko "$appdir/usr/bin/unko"
install -Dm644 assets/unko-256.png \
  "$appdir/usr/share/icons/hicolor/256x256/apps/unko.png"
install -Dm644 LICENSE "$appdir/usr/share/doc/unko/LICENSE"
install -Dm644 NOTICE "$appdir/usr/share/doc/unko/NOTICE"

# linuxdeploy populates the AppDir, bundles the dynamic libraries and writes
# AppRun; it needs the desktop file and icon at the AppDir root as well.
install -Dm644 assets/unko.desktop "$appdir/usr/share/applications/unko.desktop"
install -Dm644 assets/unko.desktop "$appdir/unko.desktop"
install -Dm644 assets/unko-256.png "$appdir/unko.png"
export LINUXDEPLOY_OUTPUT_VERSION="$version"
"$tools/linuxdeploy" --appdir "$appdir" \
  --executable "$appdir/usr/bin/unko-app" \
  --desktop-file "$appdir/usr/share/applications/unko.desktop" \
  --icon-file "$appdir/usr/share/icons/hicolor/256x256/apps/unko.png"

ARCH=$arch "$tools/appimagetool" "$appdir" "$out/unko-${version}-${arch}.AppImage"
echo "AppImage: $out/unko-${version}-${arch}.AppImage"
