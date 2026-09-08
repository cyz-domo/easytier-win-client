# EasyTier for Windows

<div align="center">
  <img src="app-icon.png" width="96" alt="EasyTier Windows Logo" />
  <p><strong>简单、轻量、去中心化的全网状 Mesh 虚拟局域网客户端（Windows 版）</strong></p>
  <p>把散落在家里、办公室、云服务器上的 Windows PC 与设备拉进同一个虚拟局域网，互访就像插在同一台交换机上。</p>
  <p>
    <a href="#-核心特性">核心特性</a> ·
    <a href="#-系统要求与安装">安装使用</a> ·
    <a href="#-双运行架构">架构设计</a> ·
    <a href="#-本地开发与构建">从源码构建</a> ·
    <a href="#-macOS-客户端特性对标">特性对标</a> ·
    <a href="#-文档导航">文档导航</a>
  </p>
</div>

---

## 🌟 核心特性

- **可视化多网络管理**：支持独立创建、配置、切换和管理多个虚拟网络，配置与状态独立隔离。
- **双模运行引擎**：
  - **便携模式 (Compat Mode)**：无需管理员安装，开箱即用，直接管理用户态 Core 进程。
  - **系统服务模式 (Windows Service)**：集成 `EasyTierService` 后台常驻，电脑开机自启、无人值守运行，即使关闭客户端窗口，网络依然稳定在线。
- **全方位状态看板**：
  - **对等节点拓扑 (Peers)**：即时查看在线节点、P2P 直连/中继路径、延迟毫秒数、隧道协议（TCP/UDP/QUIC/FakeTCP）与丢包率。
  - **智能路由表 (Routes)**：掌握整网路由代价与跃点。
  - **累计与实时流量统计**：网络重连或重启不丢失传输流量统计。
- **TOML 原生互通**：表单界面与标准 TOML 配置文件无缝双向互通，支持一键导入、导出、复制到剪贴板，格式与官方命令行及 [easytier-macos](https://github.com/socoldkiller/easytier-macos) 完全兼容。
- **远程管理与配置下发 (Remote RPC)**：可在局域网内通过 RPC 安全发现并远程修改其他 EasyTier 节点的配置。
- **自动自愈看门狗 (Watchdog)**：后台服务内置 10 秒级智能巡检与退避恢复机制，若内核因系统网络抖动异常退出，系统将自动拉起恢复。
- **系统托盘集成**：点击最小化静默运行至 Windows 任务栏通知区域，避免误关中断网络。

---

## 💻 系统要求与安装

### 系统要求
- Windows 10 (1809+) 或 Windows 11（推荐）
- 系统自带或已安装 **Microsoft Edge WebView2 Runtime**
- 架构：`x86_64` (Intel/AMD) 或 `aarch64` (ARM64)

### 安装方式
1. 前往 **[Releases](https://github.com/cyz-domo/easytier-win-client/releases)** 页面下载最新发布包。
2. **安装版 (`.exe`)**：运行安装程序，将自动配置后台常驻服务与防火墙规则。
3. **便携版 (`.zip`)**：解压至任意目录，双击 `easytier-win-client.exe` 即可直接使用。

---

## 🏗️ 双运行架构

```text
React 18 + Fluent UI (前端界面)
        │ 
        │ Tauri IPC
        ▼
Tauri Rust 适配层 (单例控制 / 权限探测 / Job Object 管理)
   ├── [便携模式] ──> 直接唤起 easytier-core.exe (管道日志收集)
   └── [服务模式] ──> Named Pipe (安全 ACL) ──> EasyTierService (SYSTEM)
                                                    │
                                                    ▼
                                            easytier-core.exe
                                  (Wintun.dll / WinDivert64.sys / Packet.dll)
```

详细全景架构思维导图与缺陷深度审计报告请参阅：[docs/architecture-audit-and-roadmap.md](docs/architecture-audit-and-roadmap.md)。

---

## 🍎 macOS 客户端特性对标

本项目在架构设计与体验交互上全面对标优秀同类实现 **[socoldkiller/easytier-macos](https://github.com/socoldkiller/easytier-macos)**：

| 能力模块 | macOS 客户端 (`easytier-macos`) | Windows 客户端 (`easytier-win-client`) | 规划进展 |
| :--- | :--- | :--- | :--- |
| **界面体系** | SwiftUI 原生窗口 | Tauri 2 (WebView2 + React 18 + TypeScript) | 已落地 |
| **后台模式** | launchd + Privileged Helper 守护进程 | Windows Service (`EasyTierService`) + SCM 调度 | 已落地 |
| **配置互通** | EasyTier 标准 TOML Document | 规范化双向 TOML 编解码器 | 已对齐 |
| **自愈恢复** | 笔记本睡眠唤醒网络自动恢复 | 服务看门狗 Watchdog 自动重连 | 规划电源广播感知 |
| **托盘速览** | 菜单栏常驻图标，点击弹出状态/IP面板 | 任务栏托盘图标，右键支持恢复/退出 | 规划轻量托盘卡片 |
| **流量监控** | 实时 1 秒级动态面积图 (Area Chart) | 节点级累计流量统计与连接速率显示 | 规划实时波形图表 |
| **远程改名** | 双击设备列表直接 RPC 远端改名 | Remote RPC 配置编辑对话框 | 优化为行内双击交互 |
| **公网发布** | HTTPS Ingress + Let's Encrypt 自动化证书 | 端口映射与 WireGuard Portal 支持 | 纳入长期规划 |

---

## 🛠️ 本地开发与构建

### 开发依赖
- **Node.js** ≥ 20 与 npm
- **Rust (MSVC)**：`x86_64-pc-windows-msvc` 工具链
- **Visual Studio Build Tools** (含 C++ MSVC 编译器与 Windows SDK)
- **protoc**（EasyTier protobuf 依赖）

### 常用命令

```powershell
# 1. 安装前端依赖
npm ci

# 2. 前端单元测试 (TOML 编解码器验证)
npm run test:codec

# 3. 前端编译检查 (tsc + vite)
npm run build

# 4. 启动本地桌面开发预览
npm run tauri dev

# 5. 本地打包一键构建 (安装包 + 便携包)
powershell -ExecutionPolicy Bypass -File scripts/build-release.ps1
```

构建排错与离线依赖配置指南详见 [docs/BUILD.md](docs/BUILD.md)。

---

## 📚 文档导航

- **[架构思维导图、缺陷审计与演进方案](docs/architecture-audit-and-roadmap.md)**：包含系统的 Mermaid 全景思维导图、代码缺陷审计、修复记录与长线规划。
- **[Windows 客户端基础设计文档](docs/windows-client-design.md)**：初代客户端协议边界与运行时规划。
- **[本地构建与打包指南](docs/BUILD.md)**：涵盖 VC-LTL、YY-Thunks 与 protoc 环境变量配置。

---

## 📄 开源许可

本项目遵循 MIT 许可证开原。EasyTier 核心及相关第三方驱动依赖遵循各自的开源许可证。
