#!/usr/bin/env bash
# Resize the committed artwork into the standard macOS icon representations.
set -euo pipefail
cd "$(dirname "$0")/.."
[[ "$(uname -s)" == Darwin ]] || { echo "需要 macOS 的 sips 和 iconutil" >&2; exit 1; }
work="$(mktemp -d "${TMPDIR:-/tmp}/yingtie-icon.XXXXXX")"
trap 'rm -rf "$work"' EXIT
iconset="$work/AppIcon.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" packaging/macos/AppIcon.png \
        --out "$iconset/icon_${size}x${size}.png" >/dev/null
    retina=$((size * 2))
    sips -z "$retina" "$retina" packaging/macos/AppIcon.png \
        --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil --convert icns --output packaging/macos/AppIcon.icns "$iconset"
echo "已生成 packaging/macos/AppIcon.icns（16–1024 像素）"
