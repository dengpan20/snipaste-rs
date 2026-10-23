# 映贴

<img src="packaging/macos/AppIcon.png" width="112" alt="映贴应用图标">

**快捷截图、图片标注与桌面贴图，让信息随手截、随手用。**

映贴（原名 Snipaste-RS）是一个以 Snipaste 的界面和操作流程为参考的开源截图工具，使用 Rust 构建原生桌面界面。你可以通过快捷键截取窗口或任意区域，添加标注、遮挡敏感内容，再复制到聊天、文档或笔记中；也可以把图片固定在桌面上，方便边看边工作。

适合日常沟通截图、问题反馈、设计取色，以及将参考资料贴在屏幕上对照使用。

[快速开始](#快速开始) · [功能](#功能) · [快捷键](#快捷键) · [开发与文档](#开发与文档)

## 为什么选择映贴

- **操作连贯**：悬停预选窗口、拖动框选、就地标注、确认复制，截图后可以直接粘贴到支持图片的应用。
- **轻量待机**：启动时只建立托盘和热键；窗口、字体和图形设备按需加载，截图结束后释放会话资源。没有贴图时，空闲 3 秒后释放图形设备。
- **原生界面**：采用 Rust、winit、egui 和 wgpu，不依赖 Electron 或 WebView。Windows 使用 DX12，初始化失败时尝试 Vulkan；Mac 使用 Metal。
- **截图之外也实用**：支持 HEX / RGB 取色、选区二维码识别和桌面贴图，减少在多个工具之间切换。
- **本机处理**：截图、标注、取色和二维码识别均在本机完成；二维码结果以文字显示，不自动打开链接。

### 内存实测

Windows 0.2.3 在一台 3440 × 1440 显示器的测试电脑上：

| 阶段 | 工作集 |
|---|---:|
| 启动待机 | 12.9 MiB |
| 连续三轮截图结束后待机 | 27.8 MiB |

以上是单机测量结果，内存占用会随屏幕尺寸、驱动和贴图数量变化。释放图形设备后的下一次截图需要重新初始化，本次测量约为 323–418 ms。完整环境、原始数据和取舍见 [内存优化记录](docs/memory.md)。

## 功能

| 功能 | 说明 |
|---|---|
| 窗口与区域截图 | 悬停预选窗口，单击确定；拖动自由框选，支持调整大小、移动选区和像素级微调 |
| 图片标注 | 矩形、椭圆、箭头、画笔、记号笔与中文文字，可调整颜色和工具属性 |
| 序号标注 | 按 N 或选择序号工具，点击依次添加圆形数字；可调整颜色和大小，支持移动、删除、撤销与重做 |
| 马赛克与橡皮擦 | 像素马赛克、模糊打码；支持框选或涂抹。橡皮擦局部恢复底图，保留区域外标注 |
| 撤销与重做 | 撤销、重做、删除选中标注和清空标注 |
| 确认与复制 | 点击勾号、按 Enter 或使用复制快捷键，合成选区图片并写入系统剪贴板 |
| 保存图片 | 系统保存对话框，支持 PNG / JPEG；取消保存可回到截图 |
| 像素取色 | 放大镜显示坐标和颜色，支持 HEX / RGB 切换与复制 |
| 二维码识别 | 点击二维码按钮或按 Q，识别当前选区内的二维码，支持多个结果和复制内容 |
| 桌面贴图 | 将选区或剪贴板图片贴到桌面，支持拖动、缩放、透明度调整、复制、保存和关闭 |
| 后台常驻 | Windows 托盘 / Mac 菜单栏、自定义截图和贴图热键、默认热键占用时尝试候选键、可选开机自启 |
| 设置面板 | 自定义截图 / 贴图快捷键、保存目录和 PNG / JPEG 默认格式，调整窗口预选与标注偏好；设置保存在本机 |

标注导出包含图片与标注，选区边框、工具条等界面元素不会进入最终图片。

## 平台与版本

当前源码与正式发布包版本：**0.2.7**。下载见 [GitHub Releases](https://github.com/dengpan20/snipaste-rs/releases/latest)。

| 平台 | 当前状态 |
|---|---|
| Windows x64 | 已完成截图主流程和桌面贴图，单元测试与原生验收通过 |
| macOS 14+ / Apple Silicon | 已在 Apple M3 / macOS 15.5 验证截图主流程、设置与序号标注；支持 macOS 14+ |
| Intel Mac / Linux | 当前不提供支持 |

Mac 每次截图覆盖**发起截图时鼠标所在的显示器**，暂不支持一次跨多个显示器框选。Windows 混合 DPI 多屏仍需更多真机验证。

## 快速开始

### Windows：下载后运行

1. 下载 [Windows 64 位便携包](https://github.com/dengpan20/snipaste-rs/releases/download/v0.2.7/yingtie-0.2.7-win64.zip)，或 [独立程序](https://github.com/dengpan20/snipaste-rs/releases/download/v0.2.7/yingtie-0.2.7.exe)。
2. 解压后运行 `snipaste-rs.exe`；独立程序可以直接运行，无需安装 Rust。
3. 程序启动后驻留系统托盘。按 **F1** 开始截图，或从托盘菜单选择“截图”。

运行新版前，请先从托盘退出旧版。发布包包含使用说明和文件校验；下载校验文件见 [SHA256SUMS.txt](https://github.com/dengpan20/snipaste-rs/releases/download/v0.2.7/SHA256SUMS.txt)。

**完成第一张截图：**

按 F1 → 拖动选择区域 → 按 Enter 确认 → 在聊天、文档或笔记中按 **Ctrl+V** 粘贴。

如 F1 已被其他软件占用，程序会尝试候选热键；菜单和截图界面显示实际生效的键。

### macOS：Apple Silicon 下载后运行

要求 **macOS 14 或更新版本**，以及 Apple Silicon 芯片。

1. 下载 [Mac arm64 应用包](https://github.com/dengpan20/snipaste-rs/releases/download/v0.2.7/yingtie-0.2.7-macos-arm64.zip)。
2. 解压后将 **映贴.app** 移入“应用程序”，然后打开；程序驻留顶部菜单栏。
3. 首次截图时，在 **系统设置 → 隐私与安全性 → 屏幕录制 / 屏幕与系统音频录制** 中允许映贴，再退出并重新打开。
4. 按 **⌘⇧2** → 拖动框选 → Enter 确认 → 在其他应用中按 **⌘V** 粘贴。

发布包由 GitHub Apple Silicon CI 构建，使用 **ad-hoc 临时签名**，尚未进行 Developer ID 签名和 Apple 公证。首次打开若系统拦截，可在“系统设置 → 隐私与安全性”中选择“仍要打开”。更新后若旧权限身份失效，请按 [Mac 指南](docs/macos.md) 刷新映贴的屏幕录制权限。

### macOS：Apple Silicon 本机构建

要求 **macOS 14 或更新版本**，以及 Apple Silicon 芯片。

1. 安装当前稳定版 [Rust](https://rustup.rs)，并安装 Apple 命令行开发工具：

   ```bash
   xcode-select --install
   ```

2. 在 GitHub 点击 **Code → Download ZIP** 获取源码并解压，或在 [Releases](https://github.com/dengpan20/snipaste-rs/releases/latest) 下载 **Source code (zip)**。
3. 双击项目根目录的 **Build macOS.command**。也可以在项目目录运行：

   ```bash
   bash scripts/build-macos.sh
   ```

4. 将生成的 `dist/macos-0.2.7/映贴.app` 移入“应用程序”，然后打开。顶部菜单栏仅显示截图图标，点击可打开截图、贴图、权限设置、自启、设置、操作说明、关于和退出菜单。
5. 按 **⌘⇧2** 或从菜单栏选择“截图”。权限提示中点击 **打开权限设置** 可直接跳转，在 **系统设置 → 隐私与安全性 → 屏幕录制 / 屏幕与系统音频录制** 中授权，再退出并重新打开应用。

构建需要 macOS 14+ SDK，首次编译会下载依赖。脚本同时生成 `dist/yingtie-0.2.7-macos-arm64.zip`，本机构建默认使用固定的本机签名证书；当前版本由 CI 构建校验；已有真机验收记录见 Mac 指南，Developer ID 签名和 Apple 公证尚未执行。更多权限、签名和验收步骤见 [Mac 指南](docs/macos.md)。

需要同时检查代码和运行单元测试时，使用 `bash scripts/build-macos.sh --verify`。桌面功能验收和三轮内存诊断分别使用 `bash scripts/verify-macos.sh`、`bash scripts/verify-macos.sh --memory`，验收应用需要自己的屏幕录制权限。

### Windows：从源码构建

安装当前稳定版 [Rust](https://rustup.rs) 和 **Visual Studio Build Tools**，勾选“使用 C++ 的桌面开发”。在开发者 PowerShell 中进入解压后的项目目录：

```powershell
rustup target add x86_64-pc-windows-msvc
cargo build --locked --release -p snipaste-app --target x86_64-pc-windows-msvc
.\target\x86_64-pc-windows-msvc\release\snipaste-rs.exe
```

现有 Windows 验收使用 GNU 工具链；使用 MinGW-w64 / w64devkit 的配置步骤见 [构建指南](docs/build.md)。

生成发布包可运行 `powershell -File scripts/package-windows.ps1 -Target x86_64-pc-windows-msvc`（GNU 环境改为 `x86_64-pc-windows-gnu`）。脚本要求源代码已提交且工作区干净，输出便携包、独立程序及带提交编号的构建信息到 `dist/release-0.2.7/`。

## 快捷键

右键点击 Windows 托盘图标（Mac 点击菜单栏图标），选择 **设置…**，点击“截图”或“桌面贴图”的按键框后按下新组合，点击 **保存** 即刻生效，并在重启后保留。支持 **恢复默认**；取消或关闭窗口会保留原设置。

F1–F24 / PrintScreen 可单独使用，普通按键需搭配 Ctrl、Alt（Mac 为 Option）或 Win（Mac 为 ⌘）。两个功能不能使用相同热键。自定义键被其他应用或系统占用时会提示保存失败，原设置保留；默认键仍沿用候选热键策略。设置窗口打开期间暂停两个全局热键，保存或关闭窗口后恢复。菜单、截图提示及操作说明显示实际生效的按键。

设置保存在 Windows 的 `%APPDATA%\Yingtie\settings.toml` 或 Mac 的 `~/Library/Application Support/Yingtie/settings.toml`；升级时兼容读取 0.2.5 的 `Snipaste-RS/hotkeys.json`。下面列出默认热键：

| 操作 | Windows | macOS |
|---|---|---|
| 开始 / 收起截图 | F1 | ⌘⇧2，F1 可用时也注册 |
| 桌面贴图 | F3 | ⌘⇧1，F3 可用时也注册 |
| 确认并复制截图 | Enter / Ctrl+C / 勾号 | Enter / ⌘C / 勾号 |
| 在其他应用中粘贴 | Ctrl+V | ⌘V |
| 保存截图 | Ctrl+S | ⌘S |
| 撤销 | Ctrl+Z | ⌘Z |
| 重做 | Ctrl+Y / Ctrl+Shift+Z | ⌘Y / ⌘⇧Z |
| 识别选区二维码 | Q | Q |
| 切换 HEX / RGB | 放大镜显示时按 Shift | 放大镜显示时按 Shift |
| 复制当前像素颜色 | 放大镜显示时按 C | 放大镜显示时按 C |
| 框选后临时取色 | 按住 Alt | 按住 Option |
| 取消截图 | Esc | Esc |

文字输入期间，Enter 用于提交文字，Shift+Enter 换行；Esc 会先关闭二维码结果、放弃文字输入或当前拖动，再取消截图。Mac 的 F1 / F3 可能需要配合 Fn 使用。

**贴图操作：**拖动移动；滚轮缩放；Shift+滚轮调整透明度；右键打开更多操作；Esc 或双击关闭。

## 设置与序号标注

Windows 从托盘菜单、Mac 从菜单栏图标打开 **设置…**。修改后点击 **保存设置** 立即应用，取消或关闭窗口会放弃未保存的修改。

- **快捷键：**点击“录入”后按下组合键，或输入 `Ctrl+Shift+S`、`Command+Shift+2`、`F1` 等格式，留空使用当前平台默认键；设置打开期间暂停全局热键，关闭后恢复。相同或被占用的自定义快捷键会提示错误并保留原快捷键。
- **保存：**选择默认文件夹与 PNG / JPEG 格式；每次保存仍可在系统保存窗口中修改文件名和格式。
- **标注：**设置颜色、笔宽、文字字号和序号大小。开启“记住上次使用”后，截图结束时保存颜色、大小、填充、笔刷和取色格式；关闭后，每次截图使用设置面板中保存的值。
- **序号：**框选后按 **N**，在选区内依次点击添加 1、2、3；切换 **V** 后拖动，按 Delete 删除。支持撤销与重做，每次新截图从 1 开始；删除中间序号不会重新排列其他数字。

设置窗口按需创建，关闭后释放窗口和字体资源，没有截图或贴图时继续沿用空闲图形资源回收机制。设置文件位于 Windows 的 `%APPDATA%\Yingtie\settings.toml` 或 Mac 的 `~/Library/Application Support/Yingtie/settings.toml`。

## 当前边界

- 暂未提供滚动截图、OCR、贴图分组与历史库。
- Windows 使用 GDI 捕获，独占全屏游戏和受保护视频内容不保证能正常截取。
- macOS 已通过当前 Retina 显示器上的自动截图与 Metal 验收，以及旧授权刷新后的独立启动截图回归；首次拒绝权限的完整交互、外接屏切换、全屏 Space、焦点恢复、系统保存面板和登录后自启仍需专项交互验收。已完成 Mac debug 诊断和 5K 屏 release 内存对比，见 [Mac 内存记录](docs/macos-memory.md)，不能套用 Windows 测量结果。

## 开发与文档

项目按职责拆分为五个 crate：

| 模块 | 职责 |
|---|---|
| `snipaste-core` | 几何、标注、样式与历史等纯逻辑 |
| `snipaste-render` | 界面主题、字体、标注绘制与图像效果 |
| `snipaste-capture` | 冻结桌面像素与选区裁剪 |
| `snipaste-platform` | 热键、剪贴板、窗口查询、保存、权限和自启 |
| `snipaste-app` | 事件循环、截图界面和贴图窗口 |

本次 macOS 通过 **161 项单元测试**、零告警代码检查，以及设置、序号与原有截图主流程的原生验收。Windows 发布流程会在 Windows x64 主机执行测试与构建；此前 0.2.5 已通过 Windows 截图与热键桌面交互验收，新序号功能仍需更多 Windows 真机验证。另有 1 项验收图片生成任务默认忽略。Mac 完整构建命令：

```bash
bash scripts/build-macos.sh --verify
```

桌面交互验收需要屏幕录制权限，使用 `bash scripts/verify-macos.sh`。结果与当前验证范围见 [Mac 验证记录](docs/macos-verification.md)。

- [构建指南](docs/build.md)
- [Mac 使用与验收](docs/macos.md)
- [截图交互与实现边界](docs/snipaste-alignment.md)
- [内存优化与测量](docs/memory.md)
- [Apple Silicon CI 工作流](.github/workflows/macos-arm64.yml)

只验收设置、序号与热键回滚，可运行 `bash scripts/verify-macos.sh --settings`；无需屏幕录制权限，使用独立配置。
