#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# 修复 rustup 的 0 字节 shim
# ---------------------------------------------------------------------------
# 症状：
#   $ cargo --version        # 无任何输出，退出码 0
#   $ ls -l ~/.cargo/bin/cargo.exe
#   -rwxr-xr-x ... 0 cargo.exe
#
# 原因：
#   rustup 在 Windows 上把 shim 建为指向 rustup.exe 的硬链接，
#   某些环境（本机即如此）下该过程产生 0 字节坏文件。
#
# 做法：
#   删除坏 shim，用 rustup.exe 重建。rustup 的 shim 本质上就是一个
#   按文件名分派的代理，只要 rustup.exe 本身完好就能重新生成。
#
# 用法：
#   bash scripts/fix-rustup-shims.sh          # 只检查，不修改
#   bash scripts/fix-rustup-shims.sh --apply  # 实际修复
# ---------------------------------------------------------------------------
set -euo pipefail

CARGO_BIN="${CARGO_BIN:-$HOME/.cargo/bin}"
RUSTUP="$CARGO_BIN/rustup.exe"
APPLY=0

for arg in "$@"; do
  case "$arg" in
    --apply) APPLY=1 ;;
    -h|--help)
      sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "未知参数：$arg（可用：--apply）" >&2; exit 2 ;;
  esac
done

if [[ ! -f "$RUSTUP" ]]; then
  echo "✗ 找不到 rustup.exe：$RUSTUP" >&2
  echo "  可用 CARGO_BIN=/path/to/.cargo/bin 覆盖默认位置" >&2
  exit 1
fi

# rustup 的 shim 集合随组件增减而变化，这里直接扫描目录里所有 0 字节的 .exe，
# 不硬编码工具名单（rustup.exe 本体除外）。
broken=()
while IFS= read -r -d '' f; do
  base=$(basename "$f")
  [[ "$base" == "rustup.exe" ]] && continue
  [[ "$base" == *.exe ]] || continue
  size=$(stat -c '%s' "$f" 2>/dev/null || echo 0)
  [[ "$size" -eq 0 ]] && broken+=("$f")
done < <(find "$CARGO_BIN" -maxdepth 1 -type f -print0)

if [[ ${#broken[@]} -eq 0 ]]; then
  echo "✓ 未发现 0 字节 shim，无需修复"
  exit 0
fi

echo "发现 ${#broken[@]} 个损坏的 shim："
for f in "${broken[@]}"; do
  echo "  - $(basename "$f") (0 字节)"
done

if [[ "$APPLY" -eq 0 ]]; then
  echo
  echo "当前为检查模式，未做任何修改。"
  echo "执行修复：bash scripts/fix-rustup-shims.sh --apply"
  exit 0
fi

echo
for f in "${broken[@]}"; do
  base=$(basename "$f")
  rm -f "$f"
  # rustup 的 shim 是按文件名（argv[0]）分派的：复制 rustup.exe 到同名文件即可
  cp "$RUSTUP" "$f"
  echo "  已重建 $base"
done

echo
echo "验证："
"$CARGO_BIN/cargo.exe" --version || {
  echo "✗ cargo 仍不可用。回退方案见 docs/build.md §5.1" >&2
  exit 1
}
echo "✓ 修复完成"
