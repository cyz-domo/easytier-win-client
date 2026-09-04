# Windows 客户端第一版设计

日期：2026-09-04
状态：待用户审阅
目标分支：`windows-client`

## 目标与范围

第一版新增 Windows 用户态桌面客户端，使用 Tauri 2、React 和 TypeScript。客户端复用现有 `core/` 目录中的 `easytier-core.exe`、`easytier-cli.exe` 及 Windows 网络组件，通过 TOML 配置、CLI JSON 和 RPC 完成配置与状态交互。

第一版不实现 Windows Service、Named Pipe、驱动安装、系统路由自动配置、系统 DNS 接管、托盘常驻和开机启动；这些能力保留为后续平台集成阶段。第一版仍需将权限不足、驱动缺失和系统集成未启用转换为明确的用户提示。

## 总体架构

```text
React + TypeScript
  ├─ 实例与配置界面
  ├─ TOML 文件/剪贴板导入导出
  ├─ Peer、Route、Stats、日志展示
  └─ 版本与诊断展示
          │ Tauri IPC
          ▼
Tauri Rust Adapter
  ├─ TOML 解析、校验与持久化
  ├─ core 进程生命周期
  ├─ easytier-cli JSON 调用
  ├─ RPC 健康检查
  ├─ runtime 版本探测
  └─ 后续 Windows Service 适配边界
          │
          ▼
 easytier-core.exe + easytier-cli.exe
```

GUI 不解析 core 的控制台文本或不稳定日志格式。所有状态数据经 Rust 层转换成结构化结果后交给前端。

## 运行时布局

开发阶段允许将运行时目录指向仓库现有 `core/`；产品布局采用版本化目录，避免覆盖正在运行的文件：

```text
EasyTier/
├── data/
│   ├── instances.json
│   └── instances/<instance-id>/
│       ├── metadata.json
│       ├── config.toml
│       ├── logs/
│       └── state.json
└── runtimes/<version>/
    ├── easytier-core.exe
    ├── easytier-cli.exe
    ├── wintun.dll
    ├── WinDivert64.sys
    └── Packet.dll
```

每个实例独立拥有 ID、名称、配置文件、RPC 端口、进程 ID、运行时版本、状态和时间戳。多个实例不得共享 RPC 端口或状态文件。

## 配置与导入导出

前端内部模型应与 macOS 的 `NetworkConfig` 字段和 TOML 编码规则保持一致。表单、文件导入、剪贴板导入和默认配置都进入同一内部模型，再由统一编码器生成 TOML。

```text
文件/剪贴板 TOML
        ↓
解析、校验、默认值补全
        ↓
预览并保存实例配置
        ↓
启动 easytier-core.exe
```

导出支持保存 TOML 文件和复制到剪贴板。网络密钥等敏感字段默认隐藏；复制或导出前必须明确提示用户。

## Core 与 CLI 生命周期

启动实例时 Rust 层依次执行：

1. 确认运行时包含 core、CLI 和所需 DLL/驱动文件；
2. 解析并校验实例 TOML；
3. 为实例选择可用的本地 RPC 端口；
4. 启动 `easytier-core.exe`，传入实例配置目录和 RPC 参数；
5. 记录 PID、端口和启动时间；
6. 等待 RPC 健康检查成功；
7. 将状态推送给前端。

状态查询优先使用 `easytier-cli.exe --output json`，覆盖 peer、route、stats、logger 等 macOS 客户端已有的信息面板。停止、异常退出、端口冲突、CLI JSON 解析失败和 core 版本不兼容都必须形成明确的结构化错误。

## 版本更新

运行时以完整版本目录为单位更新。更新流程为：下载或导入新运行时、校验签名或 SHA-256、验证 `--version`、停止旧实例、切换活动版本、启动并健康检查；失败则恢复旧版本。不得只替换单个 core 可执行文件，因为 CLI、DLL、驱动和配置/RPC 能力可能需要同步更新。

第一版实现版本探测和运行时选择接口；实际在线下载、代码签名验证和自动回滚可先提供接口与本地版本切换能力，随后完善。

## Windows 权限边界

第一版允许 Tauri Rust 进程直接启动 core，并将权限不足、TUN 创建失败或驱动访问失败显示为诊断信息。权限相关代码必须封装在独立 adapter 中，不让 React 直接执行任意命令。

后续阶段接入 Windows Service 和 Named Pipe：普通 GUI 只发送白名单操作，Service 负责管理员权限下的 core 启停、驱动、TUN、路由和 DNS；协议需预留实例 ID、操作类型、请求 ID、错误码和能力版本。

## UI 结构

视觉参考 macOS SwiftUI 客户端的简洁层级和配置分组，但使用 Windows 11 Fluent 风格基础组件。主界面包含实例导航、状态摘要、配置编辑、Peer/Route/Stats、日志和设置。基础配置默认可见，高级配置折叠展示，复刻 `ConfigEditorView` 的字段分组和依赖启用逻辑。

## 测试与验收

第一版必须验证：多实例创建和切换、配置保存、TOML 文件与剪贴板双向导入导出、配置校验、core 启停、CLI JSON 状态展示、日志读取、RPC 端口隔离、异常退出状态和 `--version` 版本识别。Rust 层对实例存储、TOML 解析、进程状态机、CLI JSON 映射和端口分配提供单元测试；前端对关键表单和状态转换提供组件测试。

## 未纳入第一版

Windows Service、Named Pipe 实现、驱动安装/卸载、系统路由与 DNS 自动修改、托盘后台运行、开机启动、在线更新服务器和生产代码签名均不属于第一版交付，但接口边界必须保持可扩展。
