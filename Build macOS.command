#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p dist
status=0
bash scripts/build-macos.sh 2>&1 | tee dist/macos-build.log || status=$?
echo
if [[ "$status" -ne 0 ]]; then
    echo "构建未完成。请按照上方提示处理后重试。日志保存在 dist/macos-build.log。"
else
    open dist
fi
read -r -p "按回车关闭窗口…" _answer
exit "$status"
