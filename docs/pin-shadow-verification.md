# 贴图阴影与任务栏修复验收

日期：2026-10-24。同步基线：`origin/main`，`5ac3169`（v0.2.7）。
环境：Windows x64、NVIDIA GeForce RTX 5070 Ti、DX12，桌面缩放 100%。

## 修改

- 为图片保留透明阴影边距：失焦为中性柔和阴影，聚焦为浅蓝色阴影。
- 按物理像素计算图片边界，阴影边距不改变原图尺寸；缩放与恢复原始大小沿用此边距。
- Windows 使用 DirectComposition 保留窗口透明度，并设置跳过任务栏和工具窗口样式。
- 焦点变化主动重绘；阴影透明度跟随图片透明度。

## 检查结果

- `cargo test --locked --workspace`：162 项通过，1 项图片生成任务按原设置忽略。
- `cargo clippy --locked --workspace --all-targets -- -D warnings`：通过。
- 原生贴图专项：通过；验证实际窗口使用透明合成、焦点事件前后的任务栏样式、两种阴影颜色与透明外边距。
- 同一绘制路径在 100%、125%、200% DPI 下离屏读回，逐像素与原图对比（允许 GPU 通道量化误差 1），全部通过。这不是跨显示器拖动的实机验收。
- 已查看生成的阴影对比图，普通阴影与浅蓝色焦点阴影均可见。
- release 构建通过；截图窗口启动检查呈现 43 帧后正常退出（退出码 0）。可运行程序位于 `dist/pin-shadow-fix/映贴.exe`。

专项验收使用独立设置和生成的棋盘格图片，不写入系统剪贴板：

```powershell
. .\scripts\env.ps1
cargo build --locked -p snipaste-app
$env:SNIPASTE_VERIFY_PINS_DIR = "$PWD\dist\verification-pin-shadow\final"
$env:SNIPASTE_SETTINGS_PATH = "$PWD\dist\verification-pin-shadow\final-settings.toml"
& .\target\x86_64-pc-windows-gnu\debug\snipaste-rs.exe
```

结果位于 `dist/verification-pin-shadow/final/pin-result.txt` 和同目录的六张图片。
可额外设置 `SNIPASTE_PREVIEW_PINS=1`，在验收通过后保留测试贴图供查看。

## 验收范围

完整原生验收另在序号标注的预览/导出像素对比处失败，尚未执行到贴图步骤：
`(230, 236)` 的预览为 `[233, 107, 109, 255]`，导出为 `[234, 109, 110, 255]`。
该结果不能记为完整截图流程通过；本次单独执行了上面的贴图专项。
Windows 界面自动化未列出工具窗口，因此焦点与任务栏检查使用原生窗口样式查询和事件验收。
macOS 本次未做实机验证。

## 保存命名与发布检查

- 加入日期和每日保存次数命名后，Windows 全工作区单元测试 166 项通过，1 项图片生成任务按原设置忽略。
- 新增验证覆盖 PNG / JPEG、不同目录、重启续计数、跨日、取消、写入失败、自定义文件名和并发保存窗口。
- Windows 全工作区零告警代码检查和 release 构建通过。
- macOS arm64 的 Rust 跨平台类型检查通过；该检查不包含 Objective-C 原生桥，原生编译和打包由发布流程在 Mac 环境完成。
