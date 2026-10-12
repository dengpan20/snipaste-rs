# 0.2.4 macOS 验证记录

日期：2026-10-12。

## Apple Silicon 原生验证

环境：Apple M3 MacBook Air（24 GB 内存）、macOS 15.5（24F74）、Apple Command Line Tools / macOS 15.4 SDK、Rust 1.99.0（b940084d7）、`aarch64-apple-darwin`。连接内建 2560 × 1664 Retina 屏和外接 5120 × 2880 屏；连接两块屏幕不等于已经覆盖所有多屏场景。

| 检查 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | 通过 |
| 原生 workspace / all-targets clippy，`-D warnings` | 通过，零告警 |
| 原生 workspace 单元测试 | 142 通过，1 项验收图片生成任务默认忽略 |
| Apple Clang 编译 Objective-C 桥 | 通过，`-Wall -Wextra -Werror` |
| release `.app` 编译、framework 链接与打包 | 通过 |
| Mach-O 架构、最低系统与 SDK | arm64；最低 macOS 14.0；SDK 15.4 |
| `Info.plist`、临时签名、ZIP 完整性 | 通过；签名为 ad-hoc |
| 原生自动验收 `scripts/verify-macos.sh` | 通过，`result.txt` 为 `PASS` |
| 4 秒桌面冒烟 | debug 186 帧、最终 release 205 帧；首帧已由 AppKit 切换到 Metal，正常退出 |
| LaunchServices 启动正式 `.app` | 应用进程正常启动；菜单栏点击与首次授权仍需交互验收 |
| 三轮 debug 内存验收 | 通过，13 条采样；每轮待机 GPU / 截图窗口均释放 |
| Developer ID 签名 / Apple 公证 | 未执行，需本机证书与凭据 |

完整构建命令为 `bash scripts/build-macos.sh --verify`，原始输出保存到 `dist/macos-build-verify.log`。

自动验收覆盖实际 ScreenCaptureKit 捕获、Metal 合成、苹方中文显示、HEX / RGB 取色和复制、矩形 / 涂抹马赛克、快速 / 单击 / 局部擦除、像素级撤销重做、预览与导出一致性、负坐标、选区二维码和文字复制、确认图片剪贴板回读、贴图创建、连续显示 / 取消、冷启动截图和 GPU 生命周期。实际运行时选中的显示器逻辑缩放为 2×，捕获尺寸为 2940 × 1912；它与面板原生分辨率不同。

本次验收修复了两个原生问题：

- **隐藏窗口首帧失败**：Metal 后端会跳过隐藏 / 遮挡窗口，原先“先渲染隐藏窗口，再显示”的流程在连续截图中失败。现在复用现有纹理离屏渲染首帧，由 AppKit 显示，首个可用 Metal 帧完成后释放临时图像。依据为本机复现和 [wgpu 的遮挡窗口处理](https://github.com/gfx-rs/wgpu/issues/8309)，通过三种非对称颜色的屏幕回读及冒烟测试验证。
- **屏幕回读颜色偏差**：ScreenCaptureKit 默认输出显示器色彩空间，与 sRGB 导出不一致。现在显式请求 sRGB 输出，再转为 sRGB RGBA。修复前测试像素期望 `(21, 64, 116)`，回读得到 `(33, 63, 112)`；修复后原有 2 级通道误差检查通过，未放宽断言。

首次完整原生验收记录为 `dist/macos-verification/20261012-020435-42761/`，其中 `result.txt` 为 `PASS`。冒烟日志为 `dist/macos-smoke.log`（debug）和 `dist/macos-release-smoke.log`（release）。内存原始数据保留在 [macos-memory-debug.csv](macos-memory-debug.csv)，使用同一调试验收 bundle，不能作为 release 内存基准：

| 阶段 | resident size | physical footprint |
|---|---:|---:|
| 启动待机 | 31.1 MiB | 9.7 MiB |
| 第一轮截图 | 327.2 MiB | 403.7 MiB |
| 第三轮结束待机 | 140.4 MiB | 22.8 MiB |

三轮冷截图初始化耗时为 307 / 225 / 241 ms。resident size 包含已驻留页面，不等于 physical footprint；资源释放也不意味着所有驻留页面立即归还。

未覆盖：首次拒绝权限后的引导、通过 LaunchServices 打开正式应用后的单独授权、真实跨屏切换与混合缩放、全屏 Space、焦点恢复后在其他应用粘贴、系统 PNG / JPEG 保存面板、登录后自启，以及其他 Mac / 系统版本。单元测试中的负坐标与混合缩放检查属于逻辑验证。手动步骤见 [Mac 指南](macos.md)。

## 映贴名称、图标和权限入口回归

同日将应用更名为 **映贴**，增加包含 16–1024 像素尺寸的 ICNS 图标。正式和验收 bundle 都声明并包含图标；原 bundle identifier 和可执行文件名保持稳定。图标源文件及内置 image_gen 的生成提示见 [图标记录](artwork.md)。

| 检查 | 结果 |
|---|---|
| `bash scripts/build-macos.sh --verify` | fmt / clippy、142 项测试、原生桥、release 编译和签名通过；日志 `dist/yingtie-build-verify.log` |
| 新名称的原生截图验收 | `dist/macos-verification/20261012-023402-49765/result.txt` 为 `PASS`；运行日志 `dist/yingtie-native-verify.log` |
| 真实 NSAlert 图标和按钮 | 同一 Objective-C 桥编译的临时 AppKit 测试应用显示新图标、“打开权限设置”和“稍后” |
| 点击“打开权限设置” | 通过 UI 点击，系统设置直接显示“录屏与系统录音”页面，包含屏幕权限列表 |
| 点击“稍后” | 提示关闭，测试进程正常退出 |
| 分享包解压、图标和签名 | `yingtie-0.2.4-macos-arm64.zip` 解压后签名有效，ICNS 可还原标准尺寸，PNG 保留透明通道 |
| 本机应用更新 | 已安装 `/Applications/映贴.app`，LaunchServices 启动后进程正常驻留；旧应用保留于 `dist/macos-backup/20261012-023436/` |

按钮交互测试未改变系统的录屏权限。它验证提示组件及跳转，不替代正式应用首次拒绝 / 重新授权的整条流程；其他待验收场景仍见上文。

## 菜单栏刘海遮挡修复

同日复现顶部入口不可见。实际状态为 `visible=1`、`windowVisible=1`，但内建屏幕逻辑宽度为 1470 点，摄像头两侧安全区域分别为 x=0–646 和 x=825–1470；入口窗口位于 x=661–731，完全落在两侧安全区域之间。记录为 `dist/menu-bar-verification/application.log`。

改为「图标＋映贴」入口，左右键均打开菜单，增加“关于映贴”和“操作说明与快捷键”。在没有已保存位置时，为 AppKit 的 `NSStatusItem Preferred Position Snipaste-RS` 初始化 0，现有位置不覆盖。该键是 AppKit 使用的保存位置约定；本机 macOS 15.5 已验证，其他系统版本仍需检查实际位置。

修复后入口位于 x=1147–1217，处于右侧安全区域，记录为 `application-fixed.log`。测试域中预设位置 300 后重新启动，数值仍为 300，入口位于 x=994–1064，记录为 `application-custom-position.log`。真实应用的关于动作经事件循环触发后，UI 验证了新图标、名称、版本和说明窗口。

最终构建日志为 `dist/yingtie-menu-build-verify.log`，覆盖 fmt / clippy、142 项测试、原生桥、release 编译与签名。截图原生回归记录 `dist/macos-verification/20261012-114839-20969/result.txt` 为 `PASS`。已更新并启动 `/Applications/映贴.app`，原版本备份于 `dist/macos-backup/20261012-114908/`，正式应用的首次位置已保存为 0。窗口外的菜单栏入口不在当前 UI 自动化工具可直接定位的窗口范围内，因此上述可见性结论基于真实 NSStatusItem 窗口位置及 NSScreen 安全区域；未声称已自动点击入口并逐项验收下拉菜单。

## 菜单栏仅图标

按用户偏好移除菜单栏名称文字，保留模板图标、悬停提示、左右键菜单及现有位置。应用说明和 README 同步改为点击“截图图标”。

`dist/yingtie-icon-only-build.log` 记录格式检查、release 编译、打包和签名通过。正式安装包的运行诊断 `dist/yingtie-icon-only-runtime.log` 确认标题为空，入口宽度为 38 点，实际横向位置为 x=1179–1217，位于屏幕右侧安全区域。此变更只移除显示文字，未重复执行此前已通过的完整截图回归。

## 重复权限提示修复

用户确认重复提示为“macOS 系统窗口＋映贴提示”。截图入口现在只调用 `CGPreflightScreenCaptureAccess` 检查状态，移除主动系统授权请求；未授权时每次运行仅提示一次，并保留跳转设置的按钮。ScreenCaptureKit 返回用户拒绝错误时直接结束该次截图，不再叠加应用弹窗。授权仍由系统逐次检查，应用不缓存授权结果。

本机另行复现了授权身份失效：系统设置中“映贴”开关为开启，使用 LaunchServices 独立启动、与正式应用同 bundle identifier 和签名证书的调试应用，读取到 `screen_recording_granted=false`，记录为 `dist/permission-standalone-verification/before-refresh.json`。该检查只读权限，不调用截图或授权请求，也不继承终端的录屏权限。

关闭并重新开启旧记录后仍读到未授权。通过系统设置移除映贴旧记录、重新添加 `/Applications/映贴.app` 后，独立启动检查读到 `screen_recording_granted=true`，记录为 `after-refresh.json`。随后修改验收 bundle 的环境配置并以同一证书重新签名，再由 LaunchServices 启动；授权仍有效，完整原生验收通过，未出现系统窗口和应用提示叠加。全程只刷新映贴的既有权限，其他应用的开关未修改。

原先 ad-hoc 签名的指定要求只包含会随构建改变的 CDHash。默认本机构建现改用同一持久本机证书，指定要求固定到证书和 bundle identifier，签名密钥保存在仓库外的专用加密钥匙串。测试修改 bundle 内容并重新签名后，CDHash 改变、指定要求保持一致，使用原要求交叉校验通过，记录为 `dist/signing-verification/result.txt`。未安装受信任根证书；未执行 Developer ID 签名或 Apple 公证。旧 ad-hoc 授权迁移到该身份仍需刷新一次系统授权。

| 检查 | 结果 |
|---|---|
| `bash scripts/build-macos.sh --verify` | fmt / clippy、145 项测试、原生桥、release 编译与签名通过；日志 `dist/yingtie-permission-build-verify.log` |
| 权限状态回归 | 新增 3 项测试通过：反复拒绝只提示一次、随后授权恢复截图、既有授权撤销后停止截图 |
| 最终构建与安装 | `dist/yingtie-permission-final-build.log`；已更新 `/Applications/映贴.app`，原版本备份于 `dist/macos-backup/20261012-130440/` |
| 签名身份稳定性 | 内容更新前后指定要求一致，严格验证通过；CI 未指定证书仍使用 ad-hoc |
| 真实独立启动授权 | 刷新旧记录后只读检查为 `true`，重新签名后仍为 `true` |
| 独立启动原生截图回归 | `dist/permission-standalone-verification/native-gui/result.txt` 为 `PASS`；覆盖实际捕获、连续显示 / 取消、确认复制、贴图、取色、马赛克、二维码和资源释放 |
| 正式应用重新打开 | `/Applications/映贴.app` 进程正常驻留；可执行文件 SHA256 与最终打包版本一致 |
| 脚本与补丁检查 | Python / Shell 语法、`git diff --check` 通过；原生代码无 `CGRequestScreenCaptureAccess` 调用 |

本次补充了既有授权失效、刷新、独立启动与重新签名后的截图回归。首次安装后的完整拒绝流程和 macOS 定期确认仍未做端到端交互验收；拒绝状态的重复提示与撤销逻辑由新增单元测试覆盖。其他未覆盖场景仍见上文。

## macOS release 内存优化回归

同日完成所用 TTC 字面读取、贴图菜单字体按需加载、首帧像素所有权转交与纹理 CPU 更新数据提前释放。GPU 结束后归还已释放的分配器缓存；仍有截图或贴图时不执行。

前后均独立启动 release bundle，在 5120 × 2880 外接屏执行三轮截图和一张 / 三张全尺寸贴图。完整峰值 footprint 从 1150.8 MiB 降至 810.7 MiB（约 30%），结束待机从 21.5 MiB 降至 14.4 MiB。原始 CSV 和测量范围见 [Mac 内存记录](macos-memory.md)。

`dist/macos-memory-final-build-verify.log` 记录 fmt / clippy、150 项测试、原生桥、release 构建及签名通过。最终独立启动的 debug 原生回归 `dist/macos-memory/native-gui/result.txt` 为 `PASS`，授权检查为 `true`；包含首次右键前不加载贴图中文字体、右键后菜单中文字形齐全的检查。查看生成的截图，中文标注与工具栏显示正常；确认 / 取消、连续截图、取色、马赛克、导出、剪贴板、二维码、贴图和资源生命周期均通过。

已安装并重新打开 `/Applications/映贴.app`，可执行文件与最终打包版本 SHA256 一致，严格签名验证通过。此前版本备份于 `dist/macos-backup/20261012-144849/`。构建后的系统钥匙串搜索列表仍只包含原先的登录钥匙串。

## 先前 Windows 回归记录

以下是此前 Windows x64 开发主机记录，不能代替 Mac 原生结果。

| 检查 | 结果 |
|---|---|
| Rust 格式检查 | 通过 |
| Windows workspace clippy，warnings 视为错误 | 通过 |
| Windows 单元测试 | 142 通过，1 项手动生成图片任务忽略 |
| Windows debug 构建 | 通过 |
| Windows 原生截图 / 显示切换 / 标注 / 剪贴板 / 二维码 / 资源生命周期验收 | 通过 |
| aarch64-apple-darwin workspace + all-targets Rust 类型与 clippy 检查 | 通过，使用 Rust-only 模式 |
| Mac 构建 / 验收脚本语法 | 通过 |
| Info.plist 的最低系统、架构、菜单栏配置 | 通过 |
| 源码包解压完整性、脚本 LF 与执行权限 | 打包时检查 |
| Apple Clang 编译 Objective-C 桥、链接 `.app` | Windows 无法验证，见上方 Mac 原生结果 |
| macOS TCC 权限、Metal 首帧、Retina、多屏、焦点与自启 | Windows 无法验证，见 Mac 原生结果及待验收项 |
| Developer ID 签名 / Apple 公证 | 未执行，需本机证书与凭据 |

Windows 上的 Rust-only 检查显式设置 `SNIPASTE_MACOS_CHECK_ONLY=1`，平台构建脚本会打印警告并跳过原生桥。这个模式没有验证 Objective-C，也不能用于产出可运行的 Mac 程序。正常 Mac 构建脚本主动取消该变量，并编译原生桥（Clang warnings 视为错误）。

新增的坐标测试覆盖负原点 Retina 显示器，以及 1× 外接屏与 2× 内屏上的跨屏窗口裁剪；LaunchAgent 测试覆盖中文、空格和 XML 特殊字符路径。原生显示切换验收还用上下不对称的像素标记检测截图翻转。

执行方式及当前单屏截图范围见 [Mac 指南](macos.md)。
