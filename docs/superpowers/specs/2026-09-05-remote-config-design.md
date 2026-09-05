# RPC 远程设备配置管理设计

日期：2026-09-05
项目：easytier-win-client
分支：feature/remote-config
参考实现：easytier-macos-main（Sources/EasyTierShared/RPC/、Rust/EasyTierGuiFFI/src/lib.rs）
状态：已按 grill-me 审计意见修订（2026-09-05）

## 1. 目标与非目标

### 目标

- 在成员列表中对在线的远端 EasyTier 节点发起"编辑配置"：读取远端实例配置（`get_config`）、编辑受限字段集、下发增量生效（`patch_config`）。
- 自动发现远端实例 UUID（`list_peer_route_pair` 按虚拟 IP 匹配 `inst_id`）。
- 本实例可被其他设备远程管理：提供"允许远程管理"开关与 RPC 白名单配置。
- 远程 RPC 通道的安全限制与 macOS 项目对齐（IP 段限制、服务方法白名单、超时与冷却）。

### 非目标

- 不做远程重启实例（`run_network_instance`）、端口转发管理、whitelist/credential 管理。因此远程可编辑字段仅限 `patch_config` 支持的增量字段集（见 §5.2）。
- 不做远程实例的创建/删除，只管理已存在的实例。
- 不改动现有本地实例管理、服务架构（windows-service / Named Pipe IPC）。
- 不支持域名形式的 RPC 地址，仅支持 IP 直连。

## 2. 技术通道

在 `src-tauri` 引入 crates.io 的 `easytier = "=2.6.4"` 依赖（与内置 core 2.6.4-8428a89d 同版本），使用其 RPC 客户端栈：

```rust
use easytier::proto::rpc_impl::standalone::StandAloneClient;
use easytier::proto::api::config::ConfigRpcClientFactory;
use easytier::proto::api::instance::PeerManageRpcClientFactory;
use easytier::tunnel::tcp::TcpTunnelConnector;
```

- `StandAloneClient::new(TcpTunnelConnector::new(url))` 建立 `tcp://<ip>:<port>` 连接；
- `scoped_client::<Factory>(domain)` 取 stub，`stub.json_call_method(BaseController::default(), method, payload_json)` 以 JSON 完成调用；
- 请求 payload 的 JSON 构造在 Rust 侧用 crate 自带类型（prost/pbjson）序列化，selector 格式（legacy `{"selector":{"Id":...}}` vs canonical `{"id":...}`）由同版本 crate 类型在编译期确定，不做运行时双发（区别于本仓库 4dbf71d 的兼容策略——那是对跨版本 CLI 的；此处 Rust 代码与 core 同版本同 crate，编译期即正确）。

参考：macOS FFI 层 `Rust/EasyTierGuiFFI/src/lib.rs` 的 `call_json_rpc_inner`（连接复用、限流、白名单）与 `call_rpc_by_service`（超时与 stub 获取），Windows 侧按同样语义实现，去掉 C ABI 外壳，直接暴露为 Tauri 命令。

## 3. Rust 侧模块 `src-tauri/src/remote_rpc.rs`

### 3.1 端点池

- `RPC_CLIENTS: DashMap<String, RpcEndpoint>`，key 为 `client_id`（`rpc-<host>-<port>`），由前端按目标节点复用。
- `RpcEndpoint` 持有：url、`Arc<Semaphore>`（每端点并发上限 2）、`tokio::sync::Mutex<StandAloneClient<TcpTunnelConnector>>`（TCP 连接跨调用复用）、冷却状态（`cooldown_until`、`last_error`）。
- 全局并发上限 4；连接超时 10s；调用超时 15s；连接失败设 30s 冷却，冷却期内直接返回上次错误。

### 3.2 安全限制（从 macOS 原样移植）

- `validate_rpc_url`：必须 `tcp://` + IP 字面量（v4/v6）+ 端口；不得含用户名密码、path、query、fragment；host 必须落在允许网段。
- `is_allowed_rpc_ip`：仅允许 `10/8`、`172.16/12`、`192.168/16`、`127/8`、`169.254/16`、`100.64/10`（EasyTier 默认虚拟网段）、`fc00::/7`、`fe80::/10`。拒绝公网 IP 与域名（防 SSRF）。
- 服务方法白名单（表驱动，精确匹配）：
  - `api.config.ConfigRpcService`：`get_config`、`patch_config`
  - `api.instance.PeerManageRpcService`：`list_peer_route_pair`（新增，用于发现远端实例 UUID，只读）
  - 其余一律拒绝。

### 3.3 Tauri 命令

```rust
#[tauri::command] async fn remote_config_discover(host: String, port: u16, virtual_ip: String)
    -> Result<RemoteInstanceInfo, String>;      // { instance_id, hostname }

#[tauri::command] async fn remote_config_load(host: String, port: u16, instance_id: String)
    -> Result<Value, String>;                    // get_config 响应（{config: {...}}）原样透传

#[tauri::command] async fn remote_config_patch(host: String, port: u16, instance_id: String, patch: Value)
    -> Result<Value, String>;                    // patch 为 §5.2 的增量字段对象
```

- `remote_config_discover`：对远端调 `list_peer_route_pair`，在返回的 pairs 中按 `route.ipv4_addr == virtual_ip` 匹配，取 `route.inst_id`（即实例 UUID）与 hostname；找不到时报"该节点未开启配置管理或虚拟 IP 不匹配"。成员表打开编辑器时先调本命令。
- `remote_config_load`：payload `{instance: <InstanceIdentifier>}`，响应含 `config` 对象，原样透传。
- `remote_config_patch`：payload `{patch: <用户增量>, instance: <InstanceIdentifier>}`，patch 内字段须先经服务方法白名单同级的手写字段校验（只接受 §5.2 列出的键，未知键拒绝），防止任意字段透传。
- 三个命令共享端点池与超时/冷却语义。

## 4. 本实例可被远程管理（服务端前提）

现状：core 启动点有三处，均绑定 `127.0.0.1`：

- `src-tauri/src/lib.rs:251`（`rpc_portal` 参数路径）
- `src-tauri/src/lib.rs:416`（内核更新恢复路径，硬编码 `127.0.0.1:{port}`）
- `src-tauri/src/runtime_manager.rs:140`（服务路径，硬编码 `127.0.0.1:{port}`）

改动：

1. 新增共享函数 `build_rpc_portal_args(instance) -> Vec<String>`：三处启动点统一调用，消除重复。
2. `InstanceConfig`（`config_store.rs`）新增字段（serde default，向后兼容旧存储）：
   - `remote_manage_enabled: bool`（默认 false）
   - `rpc_whitelist_cidrs: Vec<String>`（默认空）
3. portal 构造规则：
   - 开关关：`--rpc-portal 127.0.0.1:{port}`（现状不变）；
   - 开关开：`--rpc-portal 0.0.0.0:{port}` + `--rpc-portal-whitelist <合并白名单>`。
4. **白名单自动注入本机环回**（审计发现 3）：合并白名单 = `["127.0.0.1/32", "::1/128"] + 用户虚拟网段`。原因：本应用健康检查（`runtime_manager.rs:162` 直连 `127.0.0.1`）与状态轮询（`App.tsx` 的 CLI `--rpc-portal 127.0.0.1:{port}`）都走环回，若白名单只含虚拟网段，开启远程管理的瞬间本应用自身全部失联。
5. 用户虚拟网段必填：开关开启且 `rpc_whitelist_cidrs` 为空时拒绝保存并提示（防止裸奔到 0.0.0.0）。UI 附风险提示：绑定 `0.0.0.0` 会暴露到物理局域网，白名单只填 EasyTier 虚拟网段。

## 5. 前端

### 5.1 入口

`App.tsx` 成员表（`visiblePeers.map` 处）加"操作"列：在线的**远端**节点显示"编辑配置"按钮（`isLocal` 行不显示）。点击后打开 `RemoteConfigDialog`，流程：

1. 用该成员的虚拟 IP 调 `remote_config_discover` 取远端实例 UUID（审计发现 1：`easytier-cli` 的 peer/route JSON 均无 `inst_id` 字段——实测确认，CLI JSON 不是 macOS 的数据源；macOS 的成员 UUID 来自 FFI `collect_network_infos` 的 `peer_route_pairs[].route.inst_id`，Windows 侧等价物即 `list_peer_route_pair` RPC）；
2. 发现成功 → 继续加载配置；失败 → 展示错误与重试。

### 5.2 RemoteConfigDialog 与受限编辑器

新组件 `src/RemoteConfigDialog.tsx`（内嵌新表单 `RemoteConfigFields`，**不复用**全量 `ConfigEditor`）：

1. `remote_config_load` 拉取远端配置（loading / 失败态内联展示，可重试）；
2. **可编辑字段集 = `patch_config` 支持的增量字段**（与 macOS `InstanceConfigPatchPayload` 一致）：
   - 主机名（hostname）、虚拟 IPv4（ipv4）、IPv6 自动公网地址（ipv6_public_addr_auto）
   - 端口转发（port_forwards）
   - 子网代理（proxy_networks）、路由（routes）、出口节点（exit_nodes）
   - mapped_listeners、connectors
   - 其余字段（网络名、密钥、监听器、RPC 门户等）不在远程可编辑范围，UI 不展示或标注"需在本机修改"；
3. 编辑器初始值从 get_config 的 `config` 对象映射；跟踪 `dirty`（与加载快照对比），关闭前弹确认；
4. 保存：前端把编辑值映射回 patch 增量对象（仅包含变更字段；服务端再做一次键白名单校验），调 `remote_config_patch`；
5. 成功后重新 `remote_config_load` 刷新快照；失败展示错误并保留编辑状态。

### 5.3 允许远程管理设置

实例设置页（现有 settings 区）新增开关与虚拟网段白名单输入（CIDR 列表），写入 §4 的两个实例字段；开关开启但白名单为空时阻止保存并提示；环回地址由后端自动注入，UI 说明无需手填。

## 6. 数据流（编辑并下发一次远端配置）

```text
成员表"编辑配置"（远端节点行）
  → remote_config_discover(host, port, virtual_ip)
      → Rust: list_peer_route_pair → 匹配 ipv4_addr → {instance_id}
  → remote_config_load(host, port, instance_id)
      → Rust: get_config → {config: {...}}
  → RemoteConfigDialog / RemoteConfigFields 编辑受限字段
  → 用户保存
      → 前端生成 patch 增量（仅变更字段）
  → remote_config_patch(host, port, instance_id, patch)
      → Rust: 键白名单校验 → patch_config → 响应
  → 成功：重新 load 刷新快照；失败：展示错误，保留编辑状态
```

## 7. 测试

- Rust 单测（`remote_rpc.rs` 内 `#[cfg(test)]`）：
  - `validate_rpc_url` / `is_allowed_rpc_ip`：合法虚拟 IP/私网 IP 通过；公网 IP、域名、带凭据、非 tcp scheme 拒绝；
  - 服务方法白名单表驱动测试（含大小写敏感、未知服务拒绝）；
  - patch 键白名单校验测试（未知键、非法类型拒绝）；
  - portal 参数构造：开关开/关、白名单合并（含自动环回注入）；
  - discover 的 ipv4_addr 匹配逻辑（多 pair、无匹配）。
- 前端：`npm run build`（tsc）守门；patch 增量生成与 dirty 判断抽纯函数做单测。
- 手工联调：
  1. 本机双实例：A 开启远程管理（确认健康检查/状态轮询不受白名单影响），B 通过成员表编辑 A 的 hostname/端口转发并下发，A 侧验证生效；
  2. 真机两台各一轮 discover/load/patch；
  3. 白名单外地址访问被拒的负路径；
  4. 旧配置文件（无新字段）加载兼容性。

## 8. 风险与对策

| 风险 | 对策 |
| --- | --- |
| easytier crate 在 MSVC 下编译失败/编译时间大增 | 官方支持 Windows；锁 `=2.6.4`；里程碑 1 先 `cargo check` 验证再继续 |
| crate 与内置 core 协议不兼容 | 版本锁死 2.6.4（与 core 同版本）；payload 用 crate 类型序列化；联调覆盖 |
| `0.0.0.0` 暴露面 | 白名单强制非空 + 自动注入环回 + UI 风险提示 + 默认关 |
| 开启远程管理后本应用失联 | 白名单自动附加 127.0.0.1/32、::1/128（审计发现 3）；联调第 1 项专门验证 |
| patch 只覆盖部分配置字段 | 明示为范围决策（非目标排除重启路径）；UI 不展示不可远程修改的字段 |
| list_peer_route_pair 返回多实例/多 IP | discover 按虚拟 IP 精确匹配；无匹配时报错并引导手动检查 |

## 9. 里程碑

1. `feature/remote-config` 分支 + `remote_rpc.rs`（端点池、校验、白名单、discover/load/patch、单测），`cargo check` 通过（验证 crate 在 MSVC 编译）；
2. Tauri 命令接线 + `RemoteConfigDialog`/`RemoteConfigFields` 前端闭环（本机双实例跑通 discover/load/patch）；
3. 本实例远程管理开关 + 白名单 + `build_rpc_portal_args` 统一三处启动点 + 设置页；
4. 真机联调 + README / docs 更新，合回 `main`。
