# ---------------------------------------------------------------------------
# snipaste-rs 构建环境固化脚本（Windows / PowerShell）
#
# 用法：
#     . .\scripts\env.ps1        # 注意前面那个点，表示在当前会话里生效
#
# 作用与 scripts/env.sh 一致，见该文件的注释。
# ---------------------------------------------------------------------------

$W64Devkit = if ($env:W64DEVKIT) { $env:W64DEVKIT } else { "$HOME\tools\w64devkit\w64devkit" }
$RustBin   = if ($env:RUST_TOOLCHAIN_BIN) { $env:RUST_TOOLCHAIN_BIN } else { "$HOME\.rustup\toolchains\stable-x86_64-pc-windows-gnu\bin" }

$gcc   = Join-Path $W64Devkit "bin\x86_64-w64-mingw32-gcc.exe"
$cargo = Join-Path $RustBin  "cargo.exe"

if (-not (Test-Path $gcc)) {
    Write-Error "env.ps1: 找不到 w64devkit 的 gcc：$gcc（请设置 W64DEVKIT）"
    return
}
if (-not (Test-Path $cargo)) {
    Write-Error "env.ps1: 找不到 toolchain 里的 cargo：$cargo（请设置 RUST_TOOLCHAIN_BIN）"
    return
}

$env:PATH = (Join-Path $W64Devkit "bin") + ";" + $RustBin + ";" + "$HOME\.cargo\bin;" + $env:PATH
if (-not $env:CARGO_BUILD_TARGET) { $env:CARGO_BUILD_TARGET = 'x86_64-pc-windows-gnu' }

Write-Host "env.ps1: 已就绪"
Write-Host "  CC     = $gcc"
Write-Host "  cargo  = $cargo"
