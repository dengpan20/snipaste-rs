#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]] || { echo "需要 Apple Silicon Mac" >&2; exit 1; }
export PATH="$HOME/.cargo/bin:$PATH"
export MACOSX_DEPLOYMENT_TARGET=14.0
unset SNIPASTE_MACOS_CHECK_ONLY SNIPASTE_VERIFY_DIR SNIPASTE_VERIFY_PERMISSION_PATH SNIPASTE_MEMORY_PROFILE_DIR SNIPASTE_MEMORY_PROFILE_PINS SNIPASTE_SMOKE_MS
target=aarch64-apple-darwin
mode="${1:-}"
profile="${2:-}"
[[ $# -le 2 && ( -z "$mode" || "$mode" == --memory || "$mode" == --memory-pins ) && ( -z "$profile" || ( "$profile" == --release && -n "$mode" ) ) ]] || {
    echo "用法: bash scripts/verify-macos.sh [--memory|--memory-pins [--release]]" >&2; exit 1;
}
build_profile=debug
if [[ "$profile" == --release ]]; then
    build_profile=release
    cargo build --locked -p snipaste-app --target "$target" --release
else
    cargo build --locked -p snipaste-app --target "$target"
fi
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
dir="$PWD/dist/macos-verification/$(date +%Y%m%d-%H%M%S)-$$"
app="$PWD/dist/macos-verification/映贴 Verify.app"
mkdir -p "$dir" "$app/Contents/MacOS" "$app/Contents/Resources"
cp "target/$target/$build_profile/snipaste-rs" "$app/Contents/MacOS/snipaste-rs"
cp packaging/macos/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
chmod 755 "$app/Contents/MacOS/snipaste-rs"
sed -e "s/@VERSION@/$version/g" -e 's/io.snipaste-rs.app/io.snipaste-rs.verify/g' \
    -e 's/<string>映贴<\/string>/<string>映贴 Verify<\/string>/g' \
    packaging/macos/Info.plist > "$app/Contents/Info.plist"
/usr/bin/python3 scripts/sign-macos.py "$app"
export RUST_LOG=info
if [[ -n "$mode" ]]; then
    export SNIPASTE_MEMORY_PROFILE_DIR="$dir"
    if [[ "$mode" == --memory-pins ]]; then export SNIPASTE_MEMORY_PROFILE_PINS=1; fi
else
    export SNIPASTE_QR_FIXTURE_PATH="$dir/qr-fixture.png"
    cargo test --locked -p snipaste-app --target "$target" write_acceptance_fixture -- --ignored
    export SNIPASTE_VERIFY_DIR="$dir"
fi
echo "即将进行桌面验收，请保持鼠标在待测显示器内，暂时不要操作鼠标键盘。"
"$app/Contents/MacOS/snipaste-rs" 2>&1 | tee "$dir/application.log"
if [[ -n "$mode" ]]; then
    rows=14
    last_stage=idle-3
    if [[ "$mode" == --memory-pins ]]; then rows=18; last_stage=pin-idle; fi
    [[ -f "$dir/memory.csv" ]] && [[ "$(wc -l < "$dir/memory.csv" | tr -d ' ')" == "$rows" ]] && tail -n 1 "$dir/memory.csv" | grep -q "^$last_stage," || {
        echo "内存验收未完成。请查看 $dir/application.log，并检查调试应用的屏幕录制权限。" >&2; exit 1;
    }
else
    [[ -f "$dir/result.txt" ]] && grep -q '^PASS:' "$dir/result.txt" || {
        echo "原生验收未通过。请查看 $dir/application.log。" >&2
        echo "若是权限问题：允许映贴 Verify 录制屏幕，再重新运行本脚本。" >&2
        exit 1
    }
fi
echo "验收通过，记录保存在 $dir"
