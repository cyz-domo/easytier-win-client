# EasyTier Windows 后台服务实施方案（修订版）

日期：2026-09-05
项目：easytier-win-client
状态：根据 grill-me 审核意见修订

## 1. 目标与非目标

### 目标

- GUI 以普通用户权限运行，不因创建 TUN 网卡而反复触发 UAC。
- 首次安装 NSIS 安装包时注册并启动后台服务。
- 服务开机常驻，但默认不自动启动网络。
- 每个网络实例独立配置 `auto_start`，可在设置页和托盘切换。
- GUI 通过本机 Named Pipe 控制服务。
- 服务负责 core 启停、TUN 相关运行、内核更新和更新后的网络恢复。
- GUI 关闭到托盘后，服务和网络继续运行。

### 非目标

- 不把 GUI 注册为 Windows Service。
- 不开放 TCP 管理端口或远程控制接口。
- 不由服务直接覆盖正在运行的 service exe；service 更新只通过安装器完成。
- 不在本次提供内核历史版本回滚 UI。

## 2. 总体架构与迁移阶段

### 2.1 二进制布局

当前 `src-tauri` 改为包含两个 binary，并抽取共享模块：

```text
src-tauri/
  src/main.rs                 # Tauri GUI binary
  src/service_main.rs         # Windows Service binary
  src/runtime_manager.rs      # core 子进程与实例状态
  src/ipc.rs                  # 请求/响应/事件 schema 与 Named Pipe
  src/kernel_updater.rs       # 下载、校验、替换 core
  src/config_store.rs         # ProgramData 配置与原子持久化
```

`Cargo.toml`：

```toml
[[bin]]
name = "easytier-win-client"
path = "src/main.rs"

[[bin]]
name = "easytier-service"
path = "src/service_main.rs"
```

GUI、service、更新器共享数据结构和纯函数，但只有 service 能操作高权限 core 进程。

### 2.2 两阶段迁移

为避免两套进程管理器同时工作，按以下阶段实施：

**阶段 A：兼容桥接**

- 新增 service 和 IPC。
- GUI 启动时检查 service 状态。
- service 可用时，启停/状态/更新全部走 IPC。
- service 不可用时，GUI 暂时保留旧的直接管理模式，并在设置页明确显示“兼容模式”；兼容模式仍受现有 UAC/权限限制。
- 同一个实例禁止同时出现在 GUI 进程表和 service 进程表。

**阶段 B：默认服务模式**

- NSIS 安装版默认只走 service。
- 兼容模式只作为一次性迁移和诊断后备，不在普通 UI 中静默使用。
- 首次成功同步后，service 成为实例唯一 owner。

实例状态分为：

```text
desired_state: stopped | running
auto_start: bool
observed_state: stopped | starting | running | stopping | failed
owner: service | legacy-gui
```

`desired_state` 不从瞬时 `observed_state` 推导，避免服务重启后状态伪造。

## 3. Windows Service SCM 生命周期

### 3.1 服务配置

服务名：`EasyTierService`；显示名：`EasyTier Service`。

- 启动类型：`SERVICE_AUTO_START`。
- 错误策略：`SERVICE_ERROR_NORMAL`。
- 服务账户：优先使用受限服务账户；如果 TUN/驱动要求验证不通过，再使用 `LocalSystem`，并收紧所有配置和 IPC ACL。
- 服务启动后只监听 IPC，不启动任何 `auto_start=false` 实例。
- 服务停止时停止所有由它拥有的 core，并等待进程退出。
- 服务接收 SCM stop/shutdown 通知后，停止接受新请求，完成清理，再报告 stopped。

### 3.2 安装、升级、卸载

NSIS 安装器包含：

```text
EasyTier.exe
easytier-service.exe
core/easytier-core.exe
core/easytier-cli.exe
core/wintun.dll
core/WinDivert64.sys
core/Packet.dll
其他官方压缩包中的依赖文件
```

安装流程：

```text
停止旧 EasyTierService（若存在）
→ 写入 GUI/service/core 文件
→ sc.exe create EasyTierService binPath= "...\\easytier-service.exe"
→ 设置 description、启动类型和恢复策略
→ 启动 EasyTierService
→ 检查 service_status
```

任一步失败，安装器报告错误；已有可用版本不能被删除或覆盖成半安装状态。

升级流程：

```text
停止网络
→ 停止旧 service
→ 替换 GUI/service/core
→ 保留 ProgramData 配置
→ 注册/更新服务配置
→ 启动新 service
→ 按 desired_state/auto_start 恢复
```

卸载流程：

```text
通知 service 停止网络
→ sc.exe stop EasyTierService 并等待
→ sc.exe delete EasyTierService
→ 删除 service、GUI 和安装目录文件
→ 保留用户配置和日志，除非用户明确选择删除
```

服务 exe 被占用时，卸载器先等待并重试；失败则中止删除并明确提示，不强行留下损坏状态。

便携版不自动注册服务，设置页提供“安装后台服务”按钮，通过一次 UAC helper 注册当前目录中的 service exe；目录移动后显示路径失效并提供重新注册。

## 4. Named Pipe IPC 安全与协议

### 4.1 管道安全

管道名：`\\.\\pipe\\EasyTierService`。

服务创建管道时显式设置安全描述符，只允许：

- 当前安装/交互用户 SID；
- `BUILTIN\\Administrators`；
- `NT AUTHORITY\\SYSTEM`。

拒绝普通其他本地用户。服务连接后取得客户端 token/PID，校验调用者属于允许 SID；不相信客户端自报的用户名或角色。限制单连接请求大小为 1 MiB，单连接串行处理，服务端限制总连接数。

服务永不接受客户端传入的任意 exe 路径。core/service 路径由服务安装目录解析，并使用 canonical path 校验在允许目录内。

### 4.2 消息格式

管道使用 UTF-8、按行分隔的 JSON，一行一个请求/响应；单行最大 1 MiB。

请求：

```json
{
  "protocol_version": 1,
  "request_id": "uuid",
  "command": "start_instance",
  "payload": {
    "instance_id": "uuid",
    "config_revision": 3
  }
}
```

响应：

```json
{
  "protocol_version": 1,
  "request_id": "uuid",
  "ok": true,
  "data": {},
  "error": null
}
```

错误使用稳定错误码：`service_unavailable`、`invalid_request`、`unauthorized`、`instance_not_found`、`busy`、`core_not_found`、`port_conflict`、`operation_timeout`、`update_failed`。

长任务事件：

```json
{
  "protocol_version": 1,
  "event": "kernel_update_progress",
  "task_id": "uuid",
  "phase": "downloading",
  "downloaded_bytes": 123,
  "total_bytes": 456,
  "percent": 26,
  "message": "正在下载内核"
}
```

必须提供：

- `service_status`；
- `list_instances`；
- `sync_instance`；
- `remove_instance`；
- `start_instance`；
- `stop_instance`；
- `set_auto_start`；
- `update_kernel`；
- `get_task_status`。

请求具有幂等语义：同一 `request_id` 重试返回原结果；重复 start/stop 不产生第二个 core。更新任务拥有 `task_id`，GUI 断线重连后可查询最终状态，不能永久 loading。

## 5. 配置存储与 ACL

service 不读取 WebView localStorage。权威配置目录：

```text
%ProgramData%\\EasyTier\\
  instances.json
  service-state.json
  logs\\service.log
  backups\\
```

配置文件 ACL：

- service 账户拥有读写；
- Administrators 可维护；
- 交互用户仅通过 IPC 读写，不直接开放任意用户写权限。

GUI 首次连接 service 时迁移 `localStorage.easytier.instances.v2`：

1. 读取并校验实例 id、端口、TOML；
2. 生成 `instances.json` 临时文件；
3. fsync 后原子 rename；
4. service 返回迁移成功；
5. 原 localStorage 保留作为回退，不因失败删除。

每个实例保存：

```json
{
  "id": "uuid",
  "name": "我的网络",
  "config_toml": "...",
  "rpc_port": 15888,
  "auto_start": false,
  "desired_state": "stopped",
  "last_error": null
}
```

网络密钥只出现在受 ACL 保护的配置文件和受控 IPC payload 中；日志禁止打印完整 TOML 和密钥。

## 6. 网络启停与 TUN 权限

- GUI 不设置 `requireAdministrator`。
- service 以服务账户启动 core，并使用 `CREATE_NO_WINDOW`。
- service 是每个实例唯一进程 owner，启动前校验配置和端口，启动后记录 PID/句柄。
- core 自行退出时，service 回收句柄、设置 `observed_state=failed` 并保存错误。
- 停止操作以句柄和 PID 为准，不按进程名误杀其他 EasyTier 实例。
- TUN/驱动部署由安装器完成，service 启动时只做存在性和版本检查；缺失时禁止启动并给出明确错误。

## 7. 开机自动启动与托盘

- service 启动后默认只加载配置，不启动网络。
- 仅 `auto_start=true` 且 `desired_state=running` 的实例按顺序启动。
- 实例启动失败不阻塞其它实例；保存错误并允许 GUI 重试。
- 用户手动停止只改变当前 `desired_state`，是否保留 `auto_start` 由设置页明确控制；默认保留偏好。
- GUI 关闭按钮：隐藏窗口到托盘。
- 托盘退出：只退出 GUI，不停止 service 或网络。
- 托盘提供：打开主窗口、启停当前实例、切换 auto_start、打开设置、退出 GUI。
- service 不可用时托盘显示离线状态，禁止假报运行中。

## 8. 内核更新边界

- 更新任务由 service 执行，GUI 只提交代理选择并订阅事件。
- 只更新 `core/`；`easytier-service.exe` 的更新走安装器。
- service 记录运行前快照，拒绝并发启停。
- 下载到 ProgramData 临时目录，校验资产、架构、版本和必需文件后才替换。
- 停止 core 后备份旧 `core/`，原子替换新目录。
- 替换失败自动恢复备份；恢复失败明确标记 service degraded。
- 按快照恢复原来运行中的实例；停止的实例保持停止。
- GUI 断线不影响任务；重连后 `get_task_status` 返回结果。

## 9. 服务不可用与兼容模式

GUI 启动执行：

```text
service_status
→ 成功：进入 service mode
→ 未安装：显示安装服务
→ 已安装未运行：显示启动/修复服务
→ IPC 失败：显示明确错误，不假报网络启动
```

兼容模式仅在迁移阶段保留，并且必须显示横幅；同一实例 owner 冲突时拒绝操作，要求用户先停止旧 GUI core，再接管到 service。

## 10. 测试与验收

### 单元测试

- IPC JSON schema、版本兼容、消息大小限制。
- SID/ACL 构造与拒绝未授权用户。
- 请求幂等、重复 start/stop、任务查询。
- 配置原子写入、损坏恢复、密钥不出现在日志。
- 服务路径 canonical 校验和路径穿越拒绝。
- x64/ARM64 core 资产和驱动检查。

### 集成测试

- NSIS 安装、升级、卸载和服务恢复。
- 服务重启后默认不启网，auto_start 实例按顺序恢复。
- 普通权限 GUI 启停 TUN 网络不重复弹 UAC。
- GUI 关闭到托盘后网络继续运行。
- GUI 退出后重新连接能取回实例状态和更新任务状态。
- 更新中断、下载失败、校验失败、core 被占用、目录不可写时旧版本可用。
- service exe 更新只通过安装器完成。

### 手工验收

- x64 和 ARM64 安装包均包含 GUI、service、core 和依赖。
- 服务控制台不可见，GUI 只有一个前端窗口。
- 设置页和托盘切换 auto_start 后，重启 Windows 行为符合配置。
- 多实例同时运行、更新和恢复无端口冲突。
- 非管理员普通本地用户不能通过 Named Pipe 控制服务。

## 11. 实施顺序

1. 抽取 `runtime_manager` 和 IPC schema。
2. 增加 `easytier-service.exe` 与 SCM 生命周期。
3. 实现 Named Pipe ACL、请求/响应和 task status。
4. 实现 ProgramData 配置迁移与原子持久化。
5. GUI 增加 service client，完成阶段 A 兼容桥接。
6. 将内核更新和 core 启停迁移到 service。
7. 完成 NSIS 安装/升级/卸载脚本及 service binary 打包。
8. 完成托盘 auto_start 操作和全局状态同步。
9. 集成测试后默认切换到 service mode。
10. 最后再考虑移除 legacy GUI 直启路径。
