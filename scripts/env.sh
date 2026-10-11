#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# snipaste-rs 构建环境固化脚本（Windows / Git Bash）
#
# 用法：
#     source scripts/env.sh
#
# 作用：把「能正常链接 windows-gnu 目标」所需的三样东西放进 PATH：
#   1. w64devkit/bin      —— 提供 x86_64-w64-mingw32-gcc / ld / as / dlltool
#   2. rustup toolchain   —— 提供可用的 cargo / rustc（绕开 0 字节 rustup shim）
#   3. ~/.cargo/bin       —— 兜底
#
# 背景（详见 docs/build.md）：
#   * 本机只能走 GNU ABI，装不上 MSVC Build Tools。
#   * rustup 的 cargo.exe shim 在本机是 0 字节（损坏），必须直接调用 toolchain 里的真身。
#   * w64devkit 附带的 GCC 16 把 libgcc_eh 合并进了 libgcc.a，不再单独提供
#     libgcc_eh.a；而 rustc 仍会传 -lgcc_eh。已在 w64devkit 的 gcc 库目录里
#     补入 rustup 配套的那一份 libgcc_eh.a 解决。
# ---------------------------------------------------------------------------

# 允许外部覆盖
W64DEVKIT="${W64DEVKIT:-$HOME/tools/w64devkit/w64devkit}"
RUST_TOOLCHAIN_BIN="${RUST_TOOLCHAIN_BIN:-$HOME/.rustup/toolchains/stable-x86_64-pc-windows-gnu/bin}"

if [ ! -x "$W64DEVKIT/bin/x86_64-w64-mingw32-gcc.exe" ]; then
    echo "env.sh: 找不到 w64devkit 的 gcc：$W64DEVKIT/bin/x86_64-w64-mingw32-gcc.exe" >&2
    echo "        请设置 W64DEVKIT 指向解压后的 w64devkit 根目录。" >&2
    return 1 2>/dev/null || exit 1
fi

if [ ! -x "$RUST_TOOLCHAIN_BIN/cargo.exe" ]; then
    echo "env.sh: 找不到 toolchain 里的 cargo：$RUST_TOOLCHAIN_BIN/cargo.exe" >&2
    echo "        请设置 RUST_TOOLCHAIN_BIN。" >&2
    return 1 2>/dev/null || exit 1
fi

export PATH="$W64DEVKIT/bin:$RUST_TOOLCHAIN_BIN:$HOME/.cargo/bin:$PATH"
export CARGO_BUILD_TARGET="${CARGO_BUILD_TARGET:-x86_64-pc-windows-gnu}"

echo "env.sh: 已就绪"
echo "  CC        = $W64DEVKIT/bin/x86_64-w64-mingw32-gcc.exe"
echo "  cargo     = $RUST_TOOLCHAIN_BIN/cargo.exe"
echo "  host      = $(rustc -vV 2>/dev/null | sed -n 's/^host: //p')"
