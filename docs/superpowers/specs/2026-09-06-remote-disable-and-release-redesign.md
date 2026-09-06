# 安全禁用远程管理入口与发布流程重设计

日期：2026-09-06
项目：easytier-win-client
状态：待实现

## 1. 目标与非目标

### 目标

- 从产品入口与构建产物中安全禁用/移除远程管理功能，保留本机 Core RPC 与普通本地管理功能。
- 重构 GitHub Actions 工作流，支持 Windows AMD64 与 ARM64 的免安装包（portable）与安装包（installer），并支持可选 GitHub Release 创建。

### 非目标

- 不改动 EasyTier 核心运行（`core/`、`core-stage/`、`src-tauri/core-stage`）与本机 RPC 服务。
- 不从 Rust 源码中物理删除远程管理实现代码；改为通过 Cargo feature gate 控制编译剔除，便于回退。
- 不影响现有用户配置（`localStorage` 中的实例配置）格式与持久化。

## 2. 禁用远程管理的边界

### 2.1 前端入口

- 移除 `src/App.tsx` 中对 `RemoteConfigDialog`（即 `RemoteConfigEditor`）的引用与使用。
- 保留 `src/RemoteConfigDialog.tsx` 文件与相关纯工具函数，但不在 UI 中渲染。
- 移除 `src/icons.tsx` 中仅在远程管理入口使用的图标引用（若存在）。
- 不改动 `src/service-client.ts`、`src/status-data.ts` 等本机管理相关代码。

### 2.2 构建产物

- portable 包与 installer 包默认不包含远程管理相关的界面资源与配置入口。
- 通过 `cfg(feature = "remote-rpc")` feature gate 控制 Rust 侧远程 RPC 模块是否编译进二进制：
  - 默认构建时不启用 `remote-rpc` feature，二进制不含远程 RPC 代码与 `easytier` crate 依赖。
  - 仅当显式启用 `remote-rpc` feature 时才编译远程管理模块（用于开发与测试）。
- 本机 RPC（`status_query`、`service_request`、`start_instance` 等）不受影响。

### 2.3 Rust 侧 feature gate

- 在 `src-tauri/Cargo.toml` 中新增 `remote-rpc` feature。
- `src-tauri/src/lib.rs` 中的 `mod remote_rpc` 与相关 Tauri 命令（`remote_config_discover`、`remote_config_load`、`remote_config_patch`）用 `#[cfg(feature = "remote-rpc")]` 包裹。
- `src-tauri/src/remote_rpc.rs` 内部依赖 `easytier` crate，需确保 feature gate 也能控制该依赖的编译。可在 `Cargo.toml` 中将 `easytier` 依赖设为 `optional = true` 并绑定到 `remote-rpc` feature。
- `portal_args.rs` 中与远程管理相关的 portal 构造逻辑不受影响（这是本机 RPC 端口绑定，与远程管理入口无关）。

## 3. GitHub Actions 工作流重设计

### 3.1 参数

- `create_release: true/false`（替代原 `publish_release`），默认 `false`。
- `version: string`（替代原 `release_tag`），用于 Release 标签与产物命名；`create_release=true` 时必填。
- 保留 `branch` 参数用于分支构建。

### 3.2 构建矩阵

保留并明确：
- Windows AMD64 (`x86_64-pc-windows-msvc`)
- Windows ARM64 (`aarch64-pc-windows-msvc`)

### 3.3 打包产物

每个架构生成两类产物：
- portable：`easytier-win-client-${{ matrix.arch }}_portable.zip`（压缩完整 portable 目录）
- installer：现有 NSIS 安装包路径 `src-tauri/target/${{ matrix.target }}/release/bundle/nsis/*.exe`

### 3.4 发布逻辑

- `create_release=false`：只上传 workflow artifacts（portable + installer）到 GitHub Actions artifacts，不创建 Release。
- `create_release=true`：创建 GitHub Release，标签为 `version` 输入值，上传全部 portable 与 installer 产物。
- Release 上传文件需包含 portable zip 与 installer exe。

### 3.5 构建与打包步骤

- 保留现有 core 下载、服务二进制构建、Tauri 构建步骤。
- portable 打包：按现有逻辑压缩 portable 目录。
- installer：保留 Tauri bundle 生成 NSIS 安装包（通过 `tauri build --target ${{ matrix.target }}` 自动生成）。
- artifacts 上传路径分别指向 portable zip 与 installer exe。

## 4. 兼容性

- 现有用户配置文件格式不变；`remote_manage_enabled`、`rpc_whitelist_cidrs` 字段保留为向后兼容字段，默认关闭。
- 本地管理功能（实例启停、配置编辑、服务模式）不受影响。
- 若需启用远程管理，可在开发/测试时显式启用 `remote-rpc` feature 并重新构建。

## 5. 风险与对策

| 风险 | 对策 |
| --- | --- |
| feature gate 导致编译错误 | 先 `cargo check` 验证默认与 `remote-rpc` 两种模式 |
| ARM64 构建工具链缺失 | 工作流使用 `dtolnay/rust-toolchain` 指定 target；本地构建需安装对应目标 |
| Release 标签命名冲突 | `version` 输入由用户控制，`create_release=true` 时校验非空 |
| portable 包体积增大 | feature gate 剔除 `easytier` crate 后，二进制体积减小 |
