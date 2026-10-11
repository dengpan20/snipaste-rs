# macOS Apple Silicon（0.2.4）

目标：macOS 14+，M1 / M2 / M3 / M4 等 Apple Silicon；仅构建 `arm64`，不包含 Intel / Rosetta 版本。

## 在 Mac 上构建

1. 解压 `snipaste-rs-0.2.4-macos-arm64-source.zip`。
2. 首次准备：安装 [Rust](https://rustup.rs)，并在终端运行 `xcode-select --install` 安装 Apple 命令行开发工具。需要 macOS 14+ SDK。
3. 双击根目录的 **Build macOS.command**。首次编译会下载依赖，耗时取决于网络和设备。
4. 将生成的 `dist/macos-0.2.4/Snipaste-RS.app` 移入 **应用程序**，双击打开。程序驻留菜单栏，不显示 Dock 图标。

命令行方式：`bash scripts/build-macos.sh`。需要编译、代码检查和单元测试时运行 `bash scripts/build-macos.sh --verify`。

脚本默认使用本机临时签名。它不等于 Apple Developer ID 签名或公证。自己在本机构建后可直接使用；分享给另一台 Mac 时，系统可能提示无法验证开发者，可通过「系统设置 → 隐私与安全性 → 仍要打开」处理。

若系统拦截 `.command`，可在终端进入解压目录后运行 `bash 'Build macOS.command'`。没有要求关闭系统安全设置。

## 首次截图权限

点击菜单栏图标 → 截图，或按 **⌘⇧2**。首次会请求屏幕录制权限。

在 **系统设置 → 隐私与安全性 → 屏幕与系统音频录制**（macOS 14 中可能叫“屏幕录制”）允许 **Snipaste-RS**，然后从菜单栏退出并重新打开。菜单项 **屏幕录制权限设置…** 可直接进入该页面。

启动、贴图和复制剪贴板内容不主动请求屏幕录制权限。不需要辅助功能权限；普通组合热键使用 Carbon 注册。

## 操作

| 操作 | 快捷键 / 方式 |
|---|---|
| 截图 | ⌘⇧2；F1 可用时也注册 |
| 桌面贴图 | ⌘⇧1；F3 可用时也注册 |
| 确认并复制图片 | Enter / 勾号 / ⌘C / 选区双击 |
| 将已确认图片粘贴到其他应用 | ⌘V |
| 保存 PNG / JPEG | ⌘S，系统保存对话框 |
| 撤销 / 重做 | ⌘Z / ⌘⇧Z（也支持 ⌘Y） |
| 取色 | Shift 切换 HEX / RGB，C 复制；框选后按住 Option 临时取色 |
| 二维码 | Q / 二维码图标，识别选区内的二维码 |
| 取消 | Esc；右键可重新框选 |

Mac 键盘的 F1 / F3 可能需要按 Fn；菜单显示实际注册成功的热键。左键点击菜单栏图标打开菜单，深浅色模式由系统调整图标颜色。

## 多屏、Retina 和内存

- 每次截图覆盖**发起截图时鼠标所在的显示器**。可分别截图内屏或外接屏；暂不提供跨多个显示器的一次框选。
- Quartz 逻辑坐标按该显示器的 backing scale 转为物理像素，避免不同缩放显示器的坐标拼接错误。跨屏窗口预选裁剪到当前屏幕。
- ScreenCaptureKit 只抓单帧，不维持视频流；输出转为 sRGB RGBA，颜色值、马赛克、橡皮擦和导出共用同一份冻结像素。
- 使用 Metal，待机不创建截图窗口、GPU 设备或字体图集；没有贴图时截图结束空闲 3 秒释放 GPU。Mac 内存诊断使用真实 resident size / physical footprint，和 Windows 的 private commit 含义不同。
- 授权期间不会创建截图界面。确认 / 取消后隐藏并销毁截图会话，并尝试将焦点交还截图前的应用，方便直接 ⌘V。
- 在用户勾选开机自启时写入 `~/Library/LaunchAgents/io.snipaste-rs.app.plist`，下次登录由 LaunchServices 打开 `.app`。请先将应用放在固定位置；移动应用后取消再重新勾选自启。

## 真机验收

1. 首次拒绝权限时有清晰提示；授权、退出重开后能截图。
2. 悬停普通窗口可预选，拖拽框选与 Retina 像素对齐；左侧 / 上方外接屏也能截。
3. 连续进入、确认、Esc 取消截图，观察是否闪白、残留上一张图或抢焦点；确认后在备忘录等应用中 ⌘V。
4. 用已知纯色图比对 HEX / RGB；检查屏幕上方和下方的颜色与坐标，排除截图上下翻转。
5. 马赛克确实改变像素；橡皮擦只擦标注，撤销 / 重做与导出一致。
6. 选区包含二维码时 Q 可识别，移出二维码后不应读取整个屏幕；中文标注正常显示。
7. 系统保存面板支持 PNG / JPEG 和取消；贴图可拖动、缩放、调透明度、复制与关闭。
8. 菜单栏退出、再次打开、自启取消 / 重新开启、全屏应用所在 Space，均检查一次。
9. 活动监视器观察待机和连续截图后的内存是否回落。可用 `bash scripts/verify-macos.sh --memory` 输出 3 轮 CSV。

自动原生验收：`bash scripts/verify-macos.sh`。第一次建议先启动正式 `.app` 授权，再退出。验收程序是单独的调试 bundle，需要单独授权；若失败，授权并重新执行。结果保存在 `dist/macos-verification/`，其中必须出现 `result.txt` 的 `PASS` 才算通过；仅退出码为 0 不足以证明成功。

## 签名与公证

有 Developer ID 证书时，构建前设置 `MACOS_SIGN_IDENTITY`。若已用 `notarytool store-credentials` 创建钥匙串配置，可再设置 `MACOS_NOTARY_PROFILE`；脚本会提交公证、附加票据并验证。证书和凭据由本机钥匙串管理，不写入源码。

## 验证范围

当前开发机是 Windows：可执行 Windows 回归和 Apple Silicon Rust 分支的类型检查，无法在此链接 Apple framework 或运行 macOS 应用。原生 Objective-C 桥、Metal 显示、权限和快捷键仍需上面的 Mac 构建 / 真机验收。仓库附带 Apple Silicon CI 工作流，放入 GitHub 仓库后可编译和上传 `.app` 压缩包；CI 不代替桌面截图权限和交互验收。
