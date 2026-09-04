# EasyTier 内核手动/自动更新设计

日期：2026-09-05
项目：easytier-win-client
状态：已获用户确认

## 背景与目标

当前 Windows 客户端通过应用运行目录旁的 `core/` 目录运行 `easytier-core.exe` 和 `easytier-cli.exe`，版本由 `detect_runtime` 探测。需要增加：

- 从 EasyTier 官方 GitHub 最新正式 Release 检查内核版本；
- 直连和 GitHub 加速地址选择，并记住用户选择；
- 用户确认后更新 Windows x64/ARM64 对应的完整 core 运行时；
- 全局展示下载与替换进度；
- 更新完成后自动恢复更新前已经运行的网络实例。

“自动更新”定义为启动时后台检查并提示，不定义为静默下载或静默替换。

## 已确认的用户选择

- 版本来源：`EasyTier/EasyTier` GitHub 最新正式 Release。
- 更新范围：完整 Windows core 运行时压缩包，而不是仅两个 exe。
- 自动策略：应用启动后台检查，发现新版本后由用户点击确认更新。
- 代理选项：
  - 直连
  - `https://ghfast.top/`
  - `https://v6.gh-proxy.org/`
  - `https://hk.gh-proxy.org/`
  - `https://cdn.gh-proxy.org/`
  - `https://edgeone.gh-proxy.org/`
- 代理选择持久化到 localStorage。
- 更新时全局显示进度，更新完成后自动重启更新前正在运行的网络。

## 架构

采用“Rust 后端负责更新事务，React 前端负责展示”的边界。

### 后端模块

新增 `src-tauri/src/kernel_updater.rs`，负责：

1. 请求 GitHub latest Release 元数据；
2. 忽略 `draft` 和 `prerelease`，解析正式版本标签；
3. 根据当前 Windows 目标架构选择资产：
   - x86_64：`easytier-windows-x86_64-vX.Y.Z.zip`
   - aarch64：`easytier-windows-arm64-vX.Y.Z.zip`
4. 按代理选项将原始 GitHub 下载 URL 拼接为最终 URL；
5. 流式下载并回调进度；
6. 解压到临时目录并校验必要文件；
7. 备份和替换现有 `core/`；
8. 在失败时恢复备份并清理临时文件。

在 `src-tauri/src/lib.rs` 注册两个 Tauri command：

- `check_kernel_update`：返回当前版本、最新版本、是否可更新、资产名和错误信息；检查失败不影响网络运行。
- `update_kernel`：执行完整更新事务，接收代理地址/代理标识，向前端窗口广播进度事件。

使用应用已有 `runtime_dir()` 作为 core 目录定位依据，禁止更新到任意未验证的路径。更新目录中的文件名和目录结构必须保持与现有 `runtime_dir()`/`paths()` 约定一致。

### 前端模块

在 `App.tsx` 的设置页增加“内核更新”卡片：

- 当前版本、最新版本、上次检查时间；
- “检查更新”按钮；
- 代理选择下拉框；
- 有新版本时显示“立即更新”按钮；
- 检查失败显示可读错误和重试按钮。

新增全局 `kernelUpdate` 状态，不绑定某个 tab。顶部或内容区固定显示更新进度条，因此切换状态、成员、路由、配置和日志页时仍可见。更新期间：

- 禁止再次检查/更新；
- 禁止所有实例启动/停止；
- 禁止配置编辑和删除实例；
- 显示当前阶段、当前文件、已下载/总大小和百分比；
- 完成显示成功结果；失败显示错误和重试入口。

代理选择使用独立 localStorage key（例如 `easytier.kernel-update-proxy.v1`），没有保存凭据，也不向 URL 添加 token。

## 更新事务与数据流

### 检查

`应用启动 -> 后台 check_kernel_update -> GitHub latest API -> 过滤正式 Release -> 选择目标架构资产 -> 返回版本结果`。

检查失败只更新设置页提示，不阻塞应用启动，也不改变当前 core 或网络实例。

### 下载与校验

1. 生成原始 GitHub Release 下载地址。
2. 依据代理选项生成实际下载地址：直连使用原始地址；加速选项使用“代理根地址 + 原始 GitHub URL”。代理根地址统一去除尾部 `/`，防止出现双斜杠。
3. 流式读取响应，按 `Content-Length` 计算百分比；无总大小时显示已下载字节和不确定进度。
4. 发送 `kernel-update-progress` 事件，至少包含：
   - `phase`: `checking`/`downloading`/`extracting`/`stopping`/`installing`/`restarting`/`completed`/`failed`
   - `downloaded_bytes`、`total_bytes`、`percent`
   - `current_file`、`message`、`error`
5. 将 zip 保存到临时目录并解压到临时 core 目录。
6. 必须存在 `easytier-core.exe`、`easytier-cli.exe`，并保留压缩包中的驱动/依赖文件（如 `wintun.dll`、`WinDivert64.sys`、`Packet.dll`）。缺少核心文件、下载中断或解压失败时，现有 core 不作任何修改。
7. 通过解压目录内的 `easytier-core.exe --version` 做基本版本核验，版本不匹配时拒绝安装。

### 停止、替换和重启

1. 在更新锁下读取 `RuntimeProcesses.children`，记录所有正在运行实例的 id、配置 TOML、RPC 端口和原运行状态。
2. 停止这些实例并等待退出；停止失败则终止更新，不替换 core。
3. 将现有 `core/` 重命名为带时间戳的备份目录。
4. 将已校验的临时 core 目录重命名/移动为新的 `core/`。尽量使用同一文件系统上的 rename，避免半替换状态。
5. 重新执行 runtime 探测。
6. 按记录的实例配置依次启动原来运行中的网络；一个实例重启失败不阻塞其它实例，结果写入运行日志和最终进度消息。
7. 更新完成后清理下载临时目录；保留最近一次备份目录，暂不提供回滚 UI。
8. 替换或重启过程发生不可恢复错误时，尝试恢复备份；恢复失败必须明确提示用户手动处理，不能伪报成功。

更新命令必须有互斥锁，同一时间只允许一个更新事务。后端命令返回错误时，前端仍要收到 `failed` 事件以关闭全局 loading 状态。

## 错误处理与安全边界

- 只接受官方仓库的正式 Release 元数据和目标架构资产。
- 不使用 GitHub token，不把敏感信息写入下载 URL 或日志。
- 所有下载内容先落临时目录并校验，校验通过后才触碰现有 core。
- 目录不可写时在替换前返回明确错误，不自动模拟 UAC。
- 更新期间不允许 core 被客户端启动/停止命令并发修改。
- 网络请求、下载、解压、版本核验、文件替换、重启任一阶段失败都落 `failed`，并尽量保持原 core 可用。
- 启动检查失败不影响现有版本运行。
- 用户取消或窗口关闭时，后端应继续完成清理或安全终止事务，不留下半成品 core 目录。

## 测试与验收

### Rust 单元测试

- x86_64/aarch64 资产名选择；未知架构拒绝。
- 版本标签解析；draft/prerelease 过滤。
- 直连和六个代理地址的 URL 拼接，确认不会出现双斜杠或 token 泄露。
- 缺少 `easytier-core.exe`/`easytier-cli.exe` 时校验失败。
- 下载无总大小时进度值安全处理；百分比始终在 0-100 范围内。

### 前端测试

- 代理选择保存和恢复。
- 进度事件到 UI 状态的映射及阶段文案。
- 下载中、替换中、重启中均显示全局进度；切换 tab 不丢失。
- 完成/失败事件都能退出 loading 状态。

### 手工验收

- x64 Windows 下载并安装对应 Release，检查 core、cli、驱动文件均更新。
- ARM64 Windows 下载并安装 ARM64 资产。
- 直连和每个代理地址分别执行版本检查/下载。
- 更新前运行多个网络实例，更新后确认原运行实例自动恢复，停止的实例保持停止。
- 模拟断网、代理失败、缺文件、目录不可写，确认旧 core 不被破坏。
- 设置页启动后台检查不阻塞主界面。

## 范围外

- 不做静默自动更新。
- 不做内核降级和历史备份回滚 UI（本次仅保留最近一次备份）。
- 不做后端直连 RPC 替代 CLI 查询。
- 不在本次加入系统托盘、节点订阅、电源管理等 macOS 其它功能。
