#!/usr/bin/env bash
# Builds assets/unko.icns from assets/unko-512.png (macOS only).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
assets="$here/../assets"
set_dir=$(mktemp -d "${TMPDIR:-/tmp}/unko-icns.XXXXXX")
trap 'rm -rf "$set_dir"' EXIT

iconset="$set_dir/Unko.iconset"
mkdir -p "$iconset"
for size in 16 32 64 128 256 512; do
  sips -z $size $size "$assets/unko-512.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z $double $double "$assets/unko-512.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$assets/unko.icns"
echo "wrote $assets/unko.icns"
