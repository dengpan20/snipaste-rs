# 0.2.4 macOS 适配验证记录

日期：2026-10-12。开发主机：Windows x64。

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
| Apple Clang 编译 Objective-C 桥、链接 `.app` | 待 Mac 执行 |
| macOS TCC 权限、Metal 首帧、Retina、多屏、焦点与自启 | 待 Mac 真机验收 |
| Developer ID 签名 / Apple 公证 | 未执行，需本机证书与凭据 |

Windows 上的 Rust-only 检查显式设置 `SNIPASTE_MACOS_CHECK_ONLY=1`，平台构建脚本会打印警告并跳过原生桥。这个模式没有验证 Objective-C，也不能用于产出可运行的 Mac 程序。正常 Mac 构建脚本主动取消该变量，并编译原生桥（Clang warnings 视为错误）。

新增的坐标测试覆盖负原点 Retina 显示器，以及 1× 外接屏与 2× 内屏上的跨屏窗口裁剪；LaunchAgent 测试覆盖中文、空格和 XML 特殊字符路径。原生显示切换验收还用上下不对称的像素标记检测截图翻转。

执行方式及当前单屏截图范围见 [Mac 指南](macos.md)。
