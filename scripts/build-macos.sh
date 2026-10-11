#!/usr/bin/env bash
# Apple Silicon only. Run on macOS, using Apple's SDK; no Windows cross linking.
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != Darwin || "$(uname -m)" != arm64 ]]; then
    echo "请在 Apple Silicon Mac 的原生终端中运行（不要勾选‘使用 Rosetta 打开’）。" >&2
    exit 1
fi
major="$(sw_vers -productVersion | cut -d. -f1)"
if (( major < 14 )); then echo "需要 macOS 14 或更新版本。" >&2; exit 1; fi
if ! xcrun --sdk macosx --show-sdk-path >/dev/null 2>&1; then
    echo "请先运行 xcode-select --install，安装 Apple 命令行开发工具后重新构建。" >&2
    exit 1
fi
sdk_major="$(xcrun --sdk macosx --show-sdk-version | cut -d. -f1)"
if (( sdk_major < 14 )); then echo "请更新 Xcode 命令行工具，SDK 需要 14 或更新版本。" >&2; exit 1; fi
export PATH="$HOME/.cargo/bin:$PATH"
if ! command -v cargo >/dev/null 2>&1; then
    echo "请先从 https://rustup.rs 安装 Rust，然后重新运行本文件。" >&2
    exit 1
fi
unset SNIPASTE_MACOS_CHECK_ONLY
export MACOSX_DEPLOYMENT_TARGET=14.0
target=aarch64-apple-darwin
rustup target add "$target"
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
[[ -n "$version" ]] || { echo "无法读取版本号" >&2; exit 1; }
if [[ "${1:-}" == --verify ]]; then
    cargo fmt --all -- --check
    cargo clippy --locked --workspace --all-targets --target "$target" -- -D warnings
    cargo test --locked --workspace --target "$target"
elif [[ $# -gt 0 ]]; then
    echo "用法: bash scripts/build-macos.sh [--verify]" >&2; exit 1
fi
cargo build --locked --release -p snipaste-app --target "$target"
app="dist/macos-${version}/Snipaste-RS.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "target/$target/release/snipaste-rs" "$app/Contents/MacOS/snipaste-rs"
chmod 755 "$app/Contents/MacOS/snipaste-rs"
sed "s/@VERSION@/$version/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
archs="$(lipo -archs "$app/Contents/MacOS/snipaste-rs")"
[[ "$archs" == arm64 ]] || { echo "错误的应用架构: $archs" >&2; exit 1; }
identity="${MACOS_SIGN_IDENTITY:--}"
if [[ "$identity" == - ]]; then
    codesign --force --sign - --identifier io.snipaste-rs.app "$app"
else
    codesign --force --sign "$identity" --options runtime --timestamp "$app"
fi
codesign --verify --strict --verbose=2 "$app"
zip="dist/snipaste-rs-${version}-macos-arm64.zip"
# ditto preserves executable permissions, bundle metadata and UTF-8 filenames.
ditto -c -k --sequesterRsrc --keepParent "$app" "$zip"
if [[ -n "${MACOS_NOTARY_PROFILE:-}" ]]; then
    [[ "$identity" != - ]] || { echo "公证需要 Developer ID 签名。" >&2; exit 1; }
    xcrun notarytool submit "$zip" --keychain-profile "$MACOS_NOTARY_PROFILE" --wait
    xcrun stapler staple "$app"
    xcrun stapler validate "$app"
    spctl --assess --type execute --verbose=2 "$app"
    ditto -c -k --sequesterRsrc --keepParent "$app" "$zip"
fi
echo
echo "构建完成：$app"
echo "分享压缩包：$zip"
echo "请将 Snipaste-RS.app 移入‘应用程序’目录，然后双击打开。"
echo "首次截图需要授予屏幕录制权限。详细步骤见 docs/macos.md。"
