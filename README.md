# EasyTier Windows Client

EasyTier 的 Windows 桌面客户端（第一版），基于 Tauri 2 + React + TypeScript。

## 功能（第一版）

- 多实例管理：创建、切换、删除，实例列表与当前实例持久化
- 每实例独立 TOML 配置
- TOML 文件导入/导出、剪贴板导入/复制
- 启动/停止 `easytier-core.exe`（Tauri Rust 进程管理）
- 运行时检测（core/CLI 路径与 `--version`）
- `easytier-cli.exe --output json` 查询接口

## 开发环境

依赖：

- Node.js ≥ 18 与 npm
- Rust（msvc 工具链）：`RUSTUP_HOME=E:\app\agent-worker\rustup`，`CARGO_HOME=E:\app\agent-worker\cargo`
- Visual Studio Build Tools（MSVC v14.44 + Windows SDK 10.0.19041）：`E:\app\agent-worker\VSBuildTools`
- WebView2 Runtime（Windows 11 自带）

Cargo 使用 rsproxy.cn 镜像（见 `E:\app\agent-worker\cargo\config.toml`）。

PowerShell 环境变量：

```powershell
$env:RUSTUP_HOME = "E:\app\agent-worker\rustup"
$env:CARGO_HOME   = "E:\app\agent-worker\cargo"
$env:PATH         = "E:\app\agent-worker\cargo\bin;$env:PATH"
```

## 常用命令

```powershell
npm install        # 安装前端依赖
npm run build      # 前端构建（tsc + vite）
cargo check        # 在 src-tauri 目录下检查 Rust 侧
npx tauri dev      # 开发模式运行桌面应用
npx tauri build    # 打包 Windows 安装包
```

## 运行时

应用默认从 `core/` 目录加载运行时：

- `easytier-core.exe`
- `easytier-cli.exe`
- `wintun.dll` 等驱动组件

运行时目录结构规划见 `docs/windows-client-design.md`。

## 设计文档

见 [docs/windows-client-design.md](docs/windows-client-design.md)。
