# 构建环境搭建指南

macOS 14+ Apple Silicon 使用 `bash scripts/build-macos.sh` 或双击根目录 `Build macOS.command`，详见 [Mac 指南](macos.md)。Cargo 默认使用当前主机目标；Windows 环境脚本选择 GNU，Mac 脚本显式指定 `aarch64-apple-darwin`。

> 适用：Windows 开发机（当前环境）
> 版本：v1.0 · 2026-10-11
> 目标平台：`x86_64-pc-windows-gnu`

---

## 1. 环境要求

| 项 | 要求 | 当前环境 |
|---|---|---|
| Rust | **≥ 1.95**（egui 0.36 的 MSRV） | 1.99.0 ✅ |
| host toolchain | `stable-x86_64-pc-windows-gnu` | 已安装 ✅ |
| MinGW-w64 | **必须**，提供 binutils 的 `as.exe` | w64devkit 2.10.0 ✅ |
| Node.js | 不需要（本项目无前端构建） | — |
| MSVC | **不需要**（详见 §2） | 未安装 |

---

## 2. 为什么用 GNU 而不是 MSVC

这是本机环境决定的，不是偏好。

### 2.1 MSVC 需要的东西本机没有

Rust 在 Windows 上默认使用 MSVC ABI，但 MSVC 目标要求：

- Visual Studio Build Tools（C++ 工作负载），约 **3 ~ 7 GB**
- **需要管理员权限**才能安装

本机既没有安装，也没有管理员权限可用。

### 2.2 GNU 是官方完整支持的替代

`x86_64-pc-windows-gnu` 是 Rust 官方支持的一级目标，配合 MinGW-w64 完全可用。
本项目全部依赖（winit / egui / wgpu / tray-icon / global-hotkey / arboard）都是
Windows 侧使用 Rust 绑定，GNU 与 MSVC 的差异对本项目几乎无影响。macOS 侧另有通过 Apple Clang 编译的 Objective-C 平台桥。

### 2.3 ⚠️ 建议：条件允许时切回 MSVC

MSVC 仍是 Windows 上 Rust 的主流目标，好处是实在的：

- **原生支持 `raw-dylib`**，不需要 MinGW，也就没有 §3 那一堆坑
- **PDB 调试信息**，在 VS Code / WinDbg 里调试体验更好
- 与 Windows SDK、性能分析工具（WPA、ETW）衔接更顺

切换命令（装好 Build Tools 之后）：

```bash
rustup toolchain install stable-x86_64-pc-windows-msvc
# 然后通过 CARGO_BUILD_TARGET=x86_64-pc-windows-msvc 选择 MSVC 目标
```

两套工具链可以共存，随时切回，不必卸载。

---

## 3. 安装步骤（当前环境已完成的记录）

### 3.1 安装 Rust 工具链

```bash
# 下载 rustup-init（GNU 版，不依赖 MSVC）
curl -L -o rustup-init.exe \
  https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-gnu/rustup-init.exe

# 安装并指定 GNU host
./rustup-init.exe -y --default-host x86_64-pc-windows-gnu --profile default

# 安装实际工具链
rustup toolchain install stable-x86_64-pc-windows-gnu --profile default
rustup default stable-x86_64-pc-windows-gnu
```

### 3.2 安装 MinGW-w64（关键步骤）

**这一步不做，编译一定失败。**

```bash
# 下 w64devkit（便携版，无需管理员权限，约 67 MB）
curl -L -o w64devkit.7z.exe \
  https://github.com/skeeto/w64devkit/releases/download/v2.10.0/w64devkit-x64-2.10.0.7z.exe

# 解压（7z 自解压包，支持 -o 指定输出目录）
mkdir -p ~/tools && cd ~/tools
./w64devkit.7z.exe -y -ow64devkit
```

解压后的实际路径是 **`~/tools/w64devkit/w64devkit/bin`**（注意是两层目录）。

验证：

```bash
~/tools/w64devkit/w64devkit/bin/as.exe --version
# 应输出: GNU assembler (GNU Binutils) 2.47 ...
```

### 3.3 让构建能找到 MinGW

每次构建前需要把 w64devkit 的 bin 加进 `PATH`。项目提供了现成脚本：

```bash
# Git Bash
source scripts/env.sh
```

```powershell
# PowerShell
. .\scripts\env.ps1
```

之后正常使用 `cargo build` 即可。

---

## 4. 为什么必须要 MinGW：`as.exe` 与 `raw-dylib`

这一节解释踩过的坑，避免以后重复排查。

### 4.1 病因链

```
windows-link 等 crate 使用 #[link(kind = "raw-dylib")]
        ↓
rustc 在 windows-gnu 目标下调用 binutils 的 dlltool 生成导入库
        ↓
dlltool 生成导入库时调用 GNU 汇编器 as
        ↓
rustup 的 rust-mingw 组件**只提供 dlltool 与 ld，不提供 as**
        ↓
error: dlltool could not create import library ... CreateProcess
```

`rust-mingw` 自带的 `GCC-WARNING.txt` 写得很直白：

> gcc.exe contained in this folder cannot be used for compiling C files -
> it is only used as a linker.

也就是说 rustup 只放了"够用来链接"的最小集合，dlltool 需要的 `as` 被省略了。

**修法**：把 MinGW-w64 的 bin 放进 `PATH`，dlltool 就能按名字找到 `as.exe`。

### 4.2 试过但走不通的方案（别再重复踩）

| 方案 | 结果 | 原因 |
|---|---|---|
| `x86_64-pc-windows-gnullvm` + `rust-lld` | ❌ | 能编译，但 rustc 不把 sysroot 的 `lib/self-contained`（`libkernel32.a` 等导入库所在处）加进链接搜索路径，报 `unable to find library -lkernel32` |
| 上述 + `-Clink-self-contained=yes` | ❌ | 该写法会让 `--crate-type lib` 触发 **rustc ICE**（编译器内部错误） |
| 上述 + `-Clink-self-contained=+linker` | ❌ | `+linker` 是 unstable 选项，stable 工具链拒绝 |
| 用 `llvm-tools` 组件里的 `llvm-dlltool` 替代 | ❌ | 该组件**不包含** `llvm-dlltool.exe` |
| 用 `-Cdlltool=<路径>` 指定别的工具 | ⚠️ | 该选项确实存在（`rustc -C help` 可查），但前提是先有一个可用的替代 dlltool，而本机没有 |

> `-Cdlltool=<path>` 这个选项值得记住。将来若遇到 dlltool 相关问题，
> 它是绕过 rustup 默认选择的正规入口。

### 4.3 第二个坑：`cannot find -lgcc_eh`（本机最后一个阻塞项）

**这一步不做，`as.exe` 问题解决后依然链接失败。**

**症状**

```text
error: linking with `x86_64-w64-mingw32-gcc` failed
  = note: ld.exe: cannot find -lgcc_eh
```

**病因**

| 环节 | 事实 |
|---|---|
| rustc 对 `x86_64-pc-windows-gnu` 目标 | **固定**传 `-lgcc_eh`（历史遗留，用于 C++ 异常展开） |
| w64devkit 的 GCC **16.2.0** | 把 unwind 支持**合并进了 `libgcc.a`**，不再单独产出 `libgcc_eh.a` |
| 结果 | ld 找不到 `libgcc_eh.a` → 链接失败 |

也就是说：**新版 GCC 已经不需要这个库，但 rustc 还在要它。**

**修法（把 rustup 自带的补上）**

rustup 的 `rust-mingw` 组件里恰好附带了一份自包含的 `libgcc_eh.a`，
把它拷进 w64devkit 的 GCC 库目录即可：

```bash
SRC="$HOME/.rustup/toolchains/stable-x86_64-pc-windows-gnu/lib/rustlib/x86_64-pc-windows-gnu/lib/self-contained/libgcc_eh.a"
DST="$HOME/tools/w64devkit/w64devkit/lib/gcc/x86_64-w64-mingw32/16.2.0/"

cp "$SRC" "$DST"
```

> ⚠️ 注意 `16.2.0` 是 GCC 版本号。换 w64devkit 版本时要同步改这个目录名，
> 可用 `ls ~/tools/w64devkit/w64devkit/lib/gcc/x86_64-w64-mingw32/` 确认实际版本。

拷完再 `cargo build --workspace` 即全绿。这一步是一次性的，重装 w64devkit 才需要重做。

**为什么不用其他办法**

| 办法 | 结果 | 原因 |
|---|---|---|
| 往 Cargo 里加 `-C link-arg=-lgcc` 顶替 | ❌ | `libgcc.a` 里有 `_Unwind_Resume` 等符号定义重复，且顺序敏感，容易引发 `multiple definition` |
| `-C link-arg=-Wl,--allow-shlib-undefined` | ❌ | 这是**未定义符号**的开关，管不了**找不到库文件** |
| 降级到旧版 w64devkit（GCC 13/14） | ⚠️ | 可行但没必要牺牲新版工具链，且要重下 67 MB |

---

## 5. 已知问题与排错

### 5.1 rustup 的 shim 是 0 字节，命令行直接调用 `cargo` 无输出

**症状**

```bash
$ cargo --version     # 什么都不输出，退出码 0
$ ls -l ~/.cargo/bin/cargo.exe
-rwxr-xr-x ... 0 cargo.exe      # ← 0 字节
```

**原因**：rustup 在 Windows 上把 shim 创建为指向 `rustup.exe` 的硬链接，
本机环境下这个过程产生了 0 字节的坏文件。

**应急绕过**：直接用工具链里的可执行文件，或走 `rustup run`：

```bash
TC="$HOME/.rustup/toolchains/stable-x86_64-pc-windows-gnu/bin"
export PATH="$TC:$PATH"
cargo --version        # ✅
```

或

```bash
rustup run stable-x86_64-pc-windows-gnu cargo build
```

**修复**：删掉 0 字节的 shim，用 `rustup.exe` 重建（见 `scripts/fix-rustup-shims.sh`）。

```bash
bash scripts/fix-rustup-shims.sh          # 先看有哪些坏了（只读）
bash scripts/fix-rustup-shims.sh --apply  # 实际修复
```

脚本会扫描 `~/.cargo/bin` 下所有 0 字节的 `.exe`（不硬编码工具名单，
因为 shim 集合随组件增减变化），逐个用 `rustup.exe` 重建。
原理：rustup 的 shim 就是 `rustup.exe` 的副本，靠 `argv[0]` 分派到对应工具。

**本机状态（2026-10-11 已修复）**：一次性重建了 13 个 shim
（`cargo` / `rustc` / `cargo-clippy` / `clippy-driver` / `cargo-fmt` / `rustfmt` /
`rust-analyzer` / `cargo-miri` / `rustdoc` / `rust-gdb` / `rust-gdbgui` /
`rust-lldb` / `rls`）。修复后仅靠 `~/.cargo/bin` 即可正常使用：

```bash
$ cargo --version
cargo 1.99.0 (5f94df478 2026-08-27)
$ cargo clippy --version
clippy 0.1.99 (b940084d7e 2026-09-28)
```

**注意**：修好 shim **不意味着可以不用 `env.sh`** —— `env.sh` 解决的是另一个问题
（让 `dlltool` 找到 `as.exe`，见 §4.1）。两者互相独立，都必须处理。

### 5.2 GitHub 下载很慢

w64devkit 下载实测约 8 分钟（67 MB）。如果太慢，可以考虑：

- 使用 `--retry` 与断点续传：`curl -L -C - -o ...`
- 换用 WinLibs 的 `.zip` 版本（体积更大但可换源）

### 5.3 混合对象格式导致链接失败

如果同时装了 MSVC 与 GNU 两套工具链，**不要混用** target 目录。
本项目的 `target/` 下会按 target triple 分子目录，正常情况不会互相污染。
若出现奇怪的链接错误，先 `cargo clean` 再构建。

### 5.4 增量编译在本机必须关闭（已固化到 `Cargo.toml` 的 profile）

**症状一（噪音，每次都刷）**

```text
warning: error deleting lock file for incremental compilation session
directory `...\target\...\debug\incremental\snipaste_core-0ly1...\s-....lock`:
拒绝访问。 (os error 5)
warning: `snipaste-core` (lib) generated 1 warning
```

**症状二（致命，会把构建搞崩）**

```text
error: the compiler unexpectedly panicked. This is a bug.
error: could not compile `snipaste-platform` (lib)
```

**根因**

两个独立的触发路径，都指向同一个底层问题 —— 本机 Windows 对
`target/.../incremental/` 下的文件**锁定与删除行为异常**（大概率是实时防护
扫描占用了文件句柄），而 rustc 的增量编译恰好重度依赖这些文件的增删改：

| 触发路径 | 过程 |
|---|---|
| 单进程（噪音） | rustc 收尾时删不掉自己的 `.lock` → 报 os error 5 → 被计入 crate warning |
| 并发（致命） | 同一 `target/` 下同时跑两个 cargo 命令（例如左终端 `clippy`、右终端 `test`）→ 争抢锁 → 增量缓存写坏 → **后续构建直接 ICE** |

> ⚠️ **重点提醒**：不要在同一个项目目录下**并发**执行 cargo 命令。
> cargo 自己会加锁，但在本机环境的文件锁异常下，那个锁不可靠。
> 想并行验证就复制一份工作区，别共用一个 `target/`。

**修复（已固化，无需每次手动处理）**

⚠️ **必须用 profile 设置，不能用 `.cargo/config.toml` 的 `[env]`。**

`Cargo.toml` 里已经写死：

```toml
[profile.dev]
incremental = false

[profile.test]
incremental = false
```

**为什么不能用 `.cargo/config.toml` 的 `[env]`**（这是踩过的坑，别再改回去）：

`CARGO_INCREMENTAL` 是 **cargo 进程启动时**读取的变量，而 `.cargo/config.toml`
的 `[env]` 表是在 cargo 启动**之后**才被应用的 —— 设置得太晚。结果是
`cargo build -v` 里 rustc 命令行**仍然带着** `-C incremental=...`，增量编译
根本没关，ICE 照样发生。

自检方法（输出为空才算关掉）：

```bash
source scripts/env.sh
cargo build --workspace -v 2>&1 | grep -o '\-C incremental=[^ ]*'
```

profile 里的 `incremental = false` 才是 cargo 保证生效的写法。

关闭后两种情况都消失。代价是全量重编译 —— 本项目约 6k 行代码，
实测全量 `clippy` 仅 25 秒，完全可接受。

**若缓存已经被写坏到 ICE，先清掉再构建**

```bash
rm -rf target/x86_64-pc-windows-gnu/debug/incremental
```

> 这一步只删编译中间产物，不碰任何源码，是安全操作。

---

## 6. 日常构建命令

```bash
source scripts/env.sh          # 加载构建环境（必需）

cargo build                    # 编译全部
cargo build --release          # 发布构建
cargo test --workspace         # 跑全部测试
cargo run -p snipaste-app      # 运行程序
cargo clippy --workspace --all-targets   # 静态检查
cargo fmt --all                # 格式化
cargo fmt --all -- --check     # 只检查格式，不修改
```

### 6.1 无人值守冒烟测试

程序支持一个"跑 N 毫秒后自动退出"的冒烟模式，用于 CI / 无人值守验证：

```bash
SNIPASTE_SMOKE_MS=4000 cargo run -p snipaste-app
```

它会正常走完整链路（托盘 → 热键 → 窗口 → GPU → 首帧），
打印帧数与关键初始化日志，然后自动退出。**这是验证"改动没把程序改崩"的最快手段。**

`SNIPASTE_SMOKE_MS` 未设置时是常驻模式（正常托盘应用行为）。

### 6.2 改动后的验收清单

每次改动建议按此顺序过一遍，全绿才算完成：

| # | 命令 | 期望 |
|---|---|---|
| 1 | `cargo fmt --all -- --check` | 无差异 |
| 2 | `cargo clippy --workspace --all-targets` | **0 warning** |
| 3 | `cargo test --workspace` | 全通过（当前 142 个，另有 1 个手动验收图片生成任务默认忽略） |
| 4 | `SNIPASTE_SMOKE_MS=4000 cargo run -p snipaste-app` | 正常退出，无 panic |

> 本项目对 clippy 的要求是**零告警**，不是"能过就行"。
> 新代码引入告警时请当场修掉，否则会像滚雪球一样越积越多。

### 6.3 内存验收

发布构建后运行 `./scripts/profile-memory.ps1`。程序依次测量启动待机、三次整屏截图、四种强度的像素马赛克、带标注导出和退出后待机，然后自动退出。

报告保存到 `target/memory-after/memory.csv` 和 `run.log`。分别记录进程工作集、私有提交、两者累计峰值、截图初始化时间及图形资源状态；不写剪贴板，不裁剪工作集。待机阶段还会检查窗口和图形设备确实已释放。只在显式设置 `SNIPASTE_MEMORY_PROFILE_DIR` 时启用，普通运行不测量。

详细结果及取舍见 [内存优化记录](memory.md)。

---

## 7. macOS Apple Silicon 构建

macOS 版本必须在有 Apple SDK 的 Mac 上构建。Windows 可检查 `aarch64-apple-darwin` 的 Rust 分支，不能在本环境链接和验证 `.app`。

0.2.4 提供一键构建、arm64 `.app` 打包、临时签名，以及可选 Developer ID 签名 / 公证。已添加 `.github/workflows/macos-arm64.yml`，在 Apple Silicon runner 上编译原生平台桥、运行测试并上传压缩包。当前没有关联的 GitHub 仓库，工作流尚未执行。

构建：`bash scripts/build-macos.sh --verify`。操作与权限说明见 [macOS 指南](macos.md)。iOS 不在当前兼容范围。
