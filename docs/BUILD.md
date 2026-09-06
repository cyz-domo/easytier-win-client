# Windows 本地构建指南（便携版 + 安装版）

本文总结在本机（或给其他 agent）从零编译本项目产物的完整命令与注意事项。
CI 等价实现在 `.github/workflows/release.yml`，本地构建与它保持一致。

## 1. 前置条件

| 依赖 | 说明 |
| --- | --- |
| Node.js ≥ 20、npm | 前端构建 + Tauri CLI（`@tauri-apps/cli` 在 devDependencies） |
| Rust stable (MSVC) | `rustup default stable`；x64 构建无需额外 target |
| protoc | easytier 依赖 prost-build 生成代码，必须设置 `PROTOC` 环境变量 |
| 7z | 解压 VC-LTL（.7z 格式）；无 PATH 版可用独立 [7zr.exe](https://www.7-zip.org/a/7zr.exe)。**注意 7zr.exe 只支持 .7z，不支持 zip**，YY-Thunks-Objs.zip 请用 Windows 自带 `C:\Windows\System32\tar.exe -xf` 解压 |
| 代理（可选但基本必需） | 下载 GitHub 上的依赖与二进制时需要 |

## 2. 网络代理（PowerShell）

GitHub 直连大概率失败（connection reset），先设置代理再执行所有下载/构建命令：

```powershell
$env:HTTP_PROXY  = "http://127.0.0.1:7890"
$env:HTTPS_PROXY = "http://127.0.0.1:7890"
```

Git Bash 写法：

```bash
export HTTP_PROXY=http://127.0.0.1:7890 HTTPS_PROXY=http://127.0.0.1:7890
```

> 坑：通过代理对 GitHub release 文件做 `curl -C -` 断点续传经常产生损坏文件，
> 失败后请**删掉重新完整下载**，并用 `7zr.exe t` 校验归档。

## 3. thunk-rs 缓存（VC-LTL + YY-Thunks）

easytier 的 build script 依赖 `thunk-rs`，默认会现场从 GitHub 下载 VC-LTL-Binary.7z
和 YY-Thunks-Objs.zip，网络不稳会直接 panic 导致编译失败。解决办法是提前下载解压，
用环境变量指过去（thunk-rs 检测到 `VC_LTL` / `YY_THUNKS` 环境变量就不再下载）：

```powershell
mkdir .build-cache; cd .build-cache
# 下载（需代理）
curl -LO https://github.com/Chuyu-Team/VC-LTL5/releases/download/v5.2.2/VC-LTL-Binary.7z
curl -LO https://github.com/Chuyu-Team/YY-Thunks/releases/download/v1.1.7/YY-Thunks-Objs.zip
# 用 7zr.exe 解压 VC-LTL（.7z）；zip 用系统 tar.exe 解压
.\7zr.exe x -y -aoa -ovc-ltl VC-LTL-Binary.7z
C:\Windows\System32\tar.exe -xf YY-Thunks-Objs.zip -C yy-thunks
# （若无 yy-thunks 目录先 mkdir）

# 设置环境变量（路径按实际位置改，指向“解压后的目录”）
$env:VC_LTL    = "E:\learn\easytier-win-client\.build-cache\vc-ltl"
$env:YY_THUNKS = "E:\learn\easytier-win-client\.build-cache\yy-thunks"
```

目录结构要求（thunk-rs "win7" feature，x64）：

- `$env:VC_LTL\TargetPlatform\6.0.6000.0\lib\x64\` 存在
- `$env:YY_THUNKS\objs\x64\YY_Thunks_for_Win7.obj` 存在

## 4. PROTOC 与 cargo git

```powershell
$env:PROTOC = "E:\app\agent-worker\cargo\registry\src\rsproxy.cn-e3de039b2554c837\protoc-bin-vendored-win32-3.2.0\bin\protoc.exe"
$env:CARGO_NET_GIT_FETCH_WITH_CLI = "true"   # easytier 是 git 依赖
```

PROTOC 路径来自 `protoc-bin-vendored-win32` crate 的缓存，机器上已有；找不到就
`cargo install` 无关，直接从 crates.io 下载该 crate 解压即可。

## 5. 完整构建流程

在仓库根目录执行：

```powershell
# 0) 代理 + 上述所有环境变量已设置

# 1) 安装前端依赖 + 编解码自测
npm ci
npm run test:codec

# 2) 阶段化 EasyTier 核心（easytier-core.exe / easytier-cli.exe / wintun.dll / WinDivert64.sys / Packet.dll）
#    本地已有 src-tauri\core-stage\ 可跳过；若缺失：
#    从 https://github.com/EasyTier/EasyTier/releases/download/v2.6.4/easytier-windows-x86_64-v2.6.4.zip
#    解压其中文件到 src-tauri\core-stage\

# 3) 编译 service 后端（独立于 GUI，先编）
cargo build --release --manifest-path src-tauri/Cargo.toml --bin easytier-service

# 4) 把 service 放进打包资源目录（tauri.release.conf.json 引用它）
New-Item -ItemType Directory -Force -Path src-tauri\service-stage | Out-Null
Copy-Item src-tauri\target\release\easytier-service.exe src-tauri\service-stage\ -Force

# 5) 构建 GUI + NSIS 安装包（务必用 tauri CLI，它会自动加 custom-protocol feature）
npm exec tauri -- build --config src-tauri/tauri.release.conf.json

# 6) 组装便携包
$release = "src-tauri\target\release"
$bundle = "easytier-win-client_x64_portable"
New-Item -ItemType Directory -Force -Path "$bundle\core" | Out-Null
Copy-Item "$release\easytier-win-client.exe" "$bundle\"
Copy-Item "src-tauri\target\release\easytier-service.exe" "$bundle\"
Copy-Item "src-tauri\core-stage\*" "$bundle\core\" -Recurse -Force
Compress-Archive -Path "$bundle\*" -DestinationPath "$bundle.zip" -Force
```

## 6. 产物位置

| 产物 | 路径 |
| --- | --- |
| NSIS 安装包 | `src-tauri\target\release\bundle\nsis\*-setup.exe` |
| GUI exe | `src-tauri\target\release\easytier-win-client.exe` |
| Service exe | `src-tauri\target\release\easytier-service.exe` |
| 便携包 | `easytier-win-client_x64_portable.zip` |

## 7. 高频坑位清单（按血泪程度排序）

1. **白屏 "localhost 拒绝连接"**：用裸 `cargo build --release` 编出的 GUI 没带
   `custom-protocol` feature，release 下前端资源不内嵌。必须用
   `npx tauri build`（或 `cargo build --features tauri/custom-protocol`）。
2. **thunk-rs 下载失败 panic**：`Download libraries from ... failed` → 按第 3 节设置
   `VC_LTL` / `YY_THUNKS` 环境变量。
3. **PROTOC 未设置**：`prost-build` 报找不到 protoc → 按第 4 节。
4. **RPC 响应解压失败/Timeout**：easytier 依赖必须带 `zstd` feature
   （Cargo.toml 里 `features = ["zstd"]`，default-features = false）。
5. **crates.io 只有 easytier ≤ 2.0.3**：本项目用 git tag v2.6.4 依赖，改版本要同步
   core-stage 的核心二进制版本。
6. **xz 链接冲突**：`zip` crate 不要开 xz feature（`features = ["deflate", "time"]`）。
7. **便携包完整性**：core 目录必须含 `easytier-core.exe`、`easytier-cli.exe`、
   `wintun.dll`、`WinDivert64.sys`、`Packet.dll`，缺一运行就 core_not_found。
8. **NSIS 安装包资源**：`tauri.release.conf.json` 通过 bundle.resources 把
   `core-stage/*` 与 `service-stage/easytier-service.exe` 打进安装目录，
   service-stage 里 exe 过期会导致安装版 service 行为落后于源码。

## 8. CI 构建（GitHub Actions）

`.github/workflows/release.yml` 支持两种触发：

- 推送 `v*` tag 自动构建；
- `workflow_dispatch` 手动触发，参数：
  - `branch`：构建分支，默认 main；
  - `create_release`：true/false，是否创建 GitHub Release；
  - `version`：Release 的 tag 名（如 v0.1.0），create_release=true 时必填。

矩阵覆盖 x64 / arm64，产物为 NSIS 安装包 + 便携 zip，统一在 Artifacts 里，
create_release=true 时再发布到 Releases。

## 9. 功能开关

远程管理（RPC 远程配置）默认**编译禁用**：`remote_config_discover/load/patch` 三个
Tauri command 都被 `#[cfg(feature = "remote-rpc")]` 门控（feature 定义在
src-tauri/Cargo.toml，默认不开），前端入口也已从 App.tsx 移除
（RemoteConfigDialog.tsx 组件保留未引用）：

```powershell
# 默认（禁用远程管理）构建 = 上文流程
# 如需恢复远程管理能力（GUI 与 service 都要带 feature）：
cargo build --release --manifest-path src-tauri/Cargo.toml --features remote-rpc --bin easytier-service
Copy-Item src-tauri\target\release\easytier-service.exe src-tauri\service-stage\ -Force
npm exec tauri -- build --config src-tauri/tauri.release.conf.json -- --features remote-rpc
```

本机状态查询 / 门户参数（portal args）不受该开关影响，始终可用。
