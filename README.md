# GameLift

把已经下好的游戏搬到另一台机器：两台 Windows 机器经网线或局域网直连，内容落地后写入平台清单，目标机的启动器直接认领为已安装，不用重新下载。

[![CI](https://github.com/Spkicn/game-lan-transfer/actions/workflows/ci.yml/badge.svg)](https://github.com/Spkicn/game-lan-transfer/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![Release](https://img.shields.io/github/v/release/Spkicn/game-lan-transfer?include_prereleases)](https://github.com/Spkicn/game-lan-transfer/releases)

工具面向「两台机器之间链路很快、但互联网出口很慢」的场景，传输过程只读取源端内容，不修改源端的任何文件。

## 安装

最终用户不需要构建，直接下载安装包：

- 到 [Releases](https://github.com/Spkicn/game-lan-transfer/releases) 下载最新的 `GameLift_<版本>_x64-setup.exe`
- 安装包未做代码签名，Windows 会给出 SmartScreen 提示，选择「仍要运行」即可
- 系统要求：Windows 10 1809 及以上或 Windows 11，64 位

需要命令行时，到同一个 Release 页下载 CLI 产物，或按下方「构建」从源码构建。

## 功能

| 模块 | 说明 |
|---|---|
| 直连自举 | 识别物理以太网口并配置点对点静态地址，不设置网关，不改变机器上其他网络的默认路由；可一键还原 |
| 对端发现 | 两端都会在直连网段内广播自身信息，`peers` 列出对端、平台与可用空间；也可直接指定对端地址 |
| 游戏库扫描 | 解析 Steam 的 `libraryfolders.vdf` 与 `appmanifest_*.acf`、Epic 的 `.item` 清单，以及战网 / EA / Ubisoft / GOG 在注册表里的安装记录 |
| 分块传输 | 多连接并发拉取分块，每块附 `blake3` 摘要并逐块校验；临时共享的 SMB 路径作为兜底保留 |
| 断点续传 | 分块完成位图落盘，进程重启或断线后只补缺失分块，已完成部分不重传 |
| 差异传输 | 目标机已有旧版本时，用内容定义分块比对，只传变化的块 |
| 落盘认领 | 内容校验通过后原子改名到目标目录，再写入 `appmanifest_*.acf` 等清单文件 |
| 传输请求 | 发送方发起、接收方选目标文件夹并同意之后才落盘；拒绝或超时不会写任何内容 |
| 任意内容 | 除游戏外还可搬任意文件夹与单个文件，一次可选多项 |
| 空间预检 | 传输前校验目标盘可用空间，不足时在开始前中止并报告缺口 |
| 桌面应用 | 连接 / 内容 / 传输三阶段单窗口，界面内可配置直连地址，进度与剩余时间实时刷新 |
| 残留清理 | 不创建系统账号、不修改系统 ACL；SMB 兜底路径的临时共享随工具清理 |

## 使用

```text
gamelift scan                             扫描本机 Steam 库
gamelift nics                             列出网口，排查直连问题
gamelift link --host 1|2                  配置直连地址，需要管理员权限
gamelift unlink                           还原直连配置
gamelift peers [--seconds N]              发现直连网段里的对端
gamelift host <appid> [--dir <路径>] [--code NNNNNN] [--iface <本机IP>]
                                          源端：启动分块传输服务，无需管理员权限
gamelift recv <appid> --peer <IP> [--code NNNNNN] [--dest <路径>] [--streams N] [--force]
                                          目标端：接收并写入认领文件
gamelift serve <appid> / stop             源端：SMB 兜底，需要管理员权限
gamelift pull <appid> --peer <IP>         目标端：SMB 兜底
```

一次完整迁移：

1. 两台机器分别执行 `gamelift link`（一端 `--host 1`，另一端 `--host 2`），或用 `--iface` 指定已配置好的直连地址。
2. 任一端执行 `gamelift peers` 可以看到对端；确定角色后，源端执行 `gamelift scan` 找到目标游戏的 `appid`，再执行 `gamelift host <appid>`，工具会给出配对码与对端命令。
3. 目标端执行 `gamelift recv <appid> --peer <源端地址> --code <配对码>`。断线或中断后重跑同一条命令即可续传；`--dest` 用于本机没有 Steam 库或想换目标盘的情况。
4. 接收完成会自动写入 `appmanifest_*.acf`；按输出提示重启启动器并验证文件完整性。

`--dir <路径>` 可以把任意文件夹当作搬运对象，不限于已安装的游戏。

不想用命令行时，启动桌面应用（`pnpm --dir apps/desktop tauri dev` 或安装包）。界面按"两个端口加一根跳线"组织，一条主线三个阶段：

1. **连接**：本机端口显示当前网卡与地址，点「找对端」发现同一网段里的机器；需要点对点直连时可就地「配置直连」把本机配成 `192.168.88.1` 或 `.2`（需要管理员权限），用完可还原；只连无线网卡也能直接用。
2. **内容**：接上之后选要搬的东西——游戏库里的游戏，或任意文件夹与文件；待发清单实时显示总量。
3. **传输**：点「发送给对端」，对端界面会出现一条请求（来源、清单、总量），对端选好目标文件夹并点「同意传输」之后才开始搬运。进度、速率与剩余时间全程可见，中断后重新发起只补没传完的部分。

## 构建

要求 Windows 10 1809+ 或 Windows 11，Rust 工具链版本见 `rust-toolchain.toml`；桌面应用还需要 Node.js 20 以上与 pnpm。

```bash
# Rust
cargo build --release -p gamelift-cli
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# 桌面应用
pnpm --dir apps/desktop install
pnpm --dir apps/desktop typecheck
pnpm --dir apps/desktop test
pnpm --dir apps/desktop tauri dev     # 本地起窗口，走 Vite 开发服务器
pnpm --dir apps/desktop tauri build   # 打安装包，内嵌前端产物
```

直接用 `cargo build` 构建桌面壳时会走开发服务器地址，需要先用 `pnpm --dir apps/desktop dev` 起前端；要一个能独立运行的二进制，用 `cargo build --release -p gamelift-desktop --features custom-protocol`。

## 仓库结构

```
crates/gamelift-core/       传输引擎：网口自举、对端发现、分块传输与校验、断点续传、差异比对
crates/gamelift-launchers/  平台适配器：Steam / Epic / 战网 / EA / Ubisoft / GOG 的库扫描与清单写入
crates/gamelift-cli/        命令行客户端，依赖 core 与 launchers
apps/desktop/               Tauri 桌面应用：src-tauri 只写命令胶水层，src 是三屏界面
```

## 状态

已实现命令行客户端与传输引擎：Steam 与 Epic 库扫描、直连自举、对端发现、多连接分块传输与逐块校验、断点续传、内容差异传输、空间预检与清单认领，SMB 与 robocopy 作为兜底路径保留；桌面应用用三阶段界面覆盖同一套能力，并支持发送方发起、接收方选目录并同意的传输请求。

平台适配器覆盖 Steam、Epic、战网、EA、Ubisoft 与 GOG。其中：

- Steam 与 Epic 会把内容写回各自的清单文件，搬完即可在启动器里看到
- 战网、EA、Ubisoft 与 GOG 的库状态记在启动器自己的数据库或注册表里，GameLift 只搬内容不写这些库状态，落地后需要在对应启动器里执行一次「定位已安装文件」或「验证完整性」

### 已知限制

- 安装包未做代码签名，Windows 会给出 SmartScreen 提示，选择「仍要运行」即可
- 尚未在真机上实测：首次使用时长、千兆链路吞吐，以及 Windows 缩放 125%/150% 下的观感
- 战网、EA、Ubisoft、GOG 四个适配器按各启动器公开的注册表布局实现，尚未在有对应启动器的机器上实测
- 并发吞吐基准取决于目标机磁盘的并行读能力，需要在目标机上手动运行 `cargo test -p gamelift-core -- --ignored` 观察

后续计划：传输加密与限速。

## 参与贡献

- 开发规范：[AGENTS.md](./AGENTS.md)
- 贡献流程：[CONTRIBUTING.md](./CONTRIBUTING.md)
- 安全边界：[SECURITY.md](./SECURITY.md)

## 许可证

MIT，详见 [LICENSE](./LICENSE)。
