# 设计：TOML 可编辑 + 状态机卡死根治 + CLI 轮询轻量化

日期：2026-09-04
项目：easytier-win-client（E:\learn\easytier-win-client）
状态：已获用户批准

## 背景与问题

1. **「查看当前 TOML」只读**：组网配置页底部的 `<details>` 折叠面板（`src/App.tsx:426-429`）用 `<pre>` 只读展示由 `encodeTOML` 实时生成的 TOML，用户无法直接编辑文本。前端已有 `decodeTOML`（`src/toml-codec.ts`，导入功能在用）和 `validateConfig`（`src/network-config.ts`），具备做「编辑→校验→写回」链路的全部基础，只是没串起来。
2. **状态卡死在「正在停止…」**：实例状态（含过渡态）被整体持久化到 localStorage（`src/App.tsx:52`，key `easytier.instances.v2`），启动时原样恢复（`App.tsx:33`）。停止流程先置 `stopping`（`App.tsx:149`）再 `await invoke('stop_instance')`（`App.tsx:152`），若 Promise 挂起或进程在恢复前被杀（如强杀提权 dev 进程树），localStorage 残留 `stopping`；按钮在 `stopping` 下被禁用（`App.tsx:260`），重启应用后永远无法点击。停止分支无 try/catch 超时兜底，`toggle()` 内的 try/catch 虽有，但若 invoke 永不 resolve 则 catch 也不会触发。
3. 后端 `stop_instance`（`src-tauri/src/lib.rs:83`）为 `kill()+wait()` 幂等强杀，本身无问题；问题纯在前端状态机。
4. **CLI 轮询进程堆积**：状态轮询每周期并行 spawn 3 个 `easytier-cli.exe`（`App.tsx:65-68`，`run_cli` 见 `lib.rs:87`）。CLI 连不上 RPC 时会内部重试、挂起十几秒；若 core 实际已死而前端状态残留 `running`（问题 2 的连锁），`setInterval` 每轮照样叠加 spawn 3 个，观测到 12+ 个 CLI 进程同时堆积。CLI 是一次性命令行工具，无常驻复用形态；彻底复用需后端直连 RPC 协议（protobuf），工程量大，列为未来优化。

## 目标

- TOML 面板可直接编辑、校验、写回表单配置。
- 任何情况下前端状态机都能自愈，不会永久卡在过渡态。
- 改动范围仅前端（`src/App.tsx` + `src/App.css`），后端 `lib.rs` 不动。

## 方案（已选定）

### 1. TOML 编辑（编辑后写回表单）

把只读 `<pre>` 换成受控 `<textarea>`，以「应用更改」为唯一同步点：

- 新增状态 `tomlDraft: string | null`：
  - `null` = 未编辑，面板展示 `encodeTOML(current.config)` 实时生成结果（与现状一致）。
  - 用户开始在面板内编辑时，以当前生成的 TOML 初始化草稿。
- 「应用更改」按钮：
  1. `decodeTOML(tomlDraft)` 结构解析；
  2. `validateConfig(parsed)` 业务校验；
  3. 两关皆过 → 将解析出的 config 写回当前实例（仅替换 `config`，`name`/`rpcPort`/`status` 不动），清空草稿、写运行日志「TOML 已应用到表单」；
  4. 任一步失败 → 内联显示具体错误（含行号/字段信息），草稿保留不丢。
- 「还原」按钮：草稿重置为 `encodeTOML(current.config)`。
- 编辑期间表单与草稿互不自动覆盖，避免双向打架。
- 样式：沿用 `.toml-preview` 容器，textarea 等宽字体、固定高度、可滚动，与现有暗色主题一致。

### 2. 「正在停止…」卡死根治

- **启动自愈**：应用启动从 localStorage 恢复实例时统一修正状态：
  - `starting` / `stopping` → `stopped`（过渡态残留自愈）；
  - `running` → `stopped`（应用重启后后端子进程表必然为空，残留 running 是假状态；`stop_instance` 幂等，误留无害，但状态总览会误导）；
  - `failed` 保留（有信息量）。
- **停止超时兜底**：`toggle()` 停止分支将 `invoke('stop_instance')` 与 5 秒超时 `Promise.race`；超时或异常 → 状态置 `failed`、日志提示「停止超时，可再次点击重试」。后端幂等，超时后即使实际已完成也无需回滚。
- 启动分支不动（已有 1200ms 后 `wait_for_exit` 秒退检测）。

### 3. CLI 轮询轻量化

双层硬性限流，保证任意时刻同时存活的 CLI 进程 ≤ 3：

- **后端 `run_cli` 超时**（`lib.rs:87`）：改为「spawn → 5 秒内未退出则 `kill()` → 返回错误」；用 `wait_timeout` crate（已是成熟零依赖方案，纯 WinAPI 等待）实现有界等待，超时后杀进程并返回 `Err("查询超时…")`。防止挂死的 CLI 进程永生。
- **前端轮询防重叠**（`App.tsx:60-82`）：`setInterval` 改为自调度 `setTimeout` 链——上一轮 `refresh` 未完成则不排下一轮；每轮内仍 `Promise.all` 3 个查询（并发 3 个 CLI，属预期）。

### 4. 当前卡住实例的解锁

修复生效后（重新 `npx tauri dev`），启动自愈自动把残留 `stopping` 重置为 `stopped`。后备手段（仅前述失效时）：WebView 开发者工具控制台执行 `localStorage.removeItem('easytier.instances.v2')` 后刷新（会重置为默认实例）。

## 改动范围

- 前端：`src/App.tsx`（TOML 面板、状态自愈、停止超时、轮询防重叠）+ `src/styles.css`（textarea 样式）
- 后端：`src-tauri/src/lib.rs`（`run_cli` 超时；新增 `wait_timeout` 依赖）+ `src-tauri/Cargo.toml`
- 其余后端命令不动。

## 数据流

- TOML 面板：`current.config --encodeTOML--> textarea 草稿 --decodeTOML+validate--> config 写回实例`；与导入文件/剪贴板导入共用同一套编解码与校验函数，行为一致。
- 状态机：`stopped ⇄ starting → running ⇄ stopping → stopped`，异常落 `failed`；`failed` 可直接再次点击按钮重试（现行行为，保持）；应用启动时按上述规则修正残留态。

## 错误处理

- TOML 解析失败：内联红字提示（含错误位置），不写回、不清草稿。
- 业务校验失败：同上，显示第一条错误消息（与启动前校验一致）。
- 停止超时/失败：状态 `failed` + 日志 + alert，可重试。
- `run_cli` 超时：返回 `Err`，前端既有 `catch → setStatusError` 链路展示「状态查询失败」，轮询继续。
- localStorage 损坏：现有 `load()` 已有 try/catch 兜底（回退默认实例），不动。

## 测试与验证

- `npm run test:codec`：确认编解码无回归（本次不改 toml-codec.ts，应全绿）。
- TOML 手工验证：编辑→故意语法错误→确认报错且草稿保留；改正→应用→重新展开面板确认 TOML 与表单一致；应用后启动网络确认配置真实生效。
- 状态机手工验证：往 localStorage 塞 `status:"stopping"`（及 `running`）→ 重启应用 → 自愈为 `stopped` 可点击；运行中点停止 → 回到 `stopped`；模拟停止超时（临时断言/埋点或代码走查）→ 落 `failed` 可重试。
- CLI 轻量化验证：运行中正常轮询（成员/路由数据照常刷新）；core 被外部杀死后，确认 CLI 进程 5 秒内被回收、状态查询报错展示、不再无限堆积进程；`cargo check` 通过。
- `tsc` 编译通过（`npm run build` 的 `tsc && vite build`）。

## 不做的事（YAGNI）

- 不做 TOML 语法高亮、行号、自动补全（无需引入编辑器库）。
- 不做 TOML 保存为正式配置文件（后端持久化），用户已明确选「写回表单」。
- 不做后端直连 RPC/protobuf 常驻连接（彻底的进程复用，工程量大，列为未来优化）。
- 不动 `magic_dns_zone` 丢弃等既有小问题（独立议题）。
