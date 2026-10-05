# GameLift

GameLift 是一个 Windows 局域网游戏迁移工具。它把一台机器上已经安装好的游戏，经点对点局域网连接迁移到另一台机器，并在迁移完成后写入平台清单文件，使目标机的启动器将游戏识别为已安装，无需重新下载。

工具面向"两台机器之间存在高速链路、但互联网出口带宽有限"的场景。传输过程只读取源端内容，不修改源端的任何文件。

## 功能

| 模块 | 说明 |
|---|---|
| 直连自举 | 识别物理以太网口并配置点对点静态地址，不设置网关，不改变机器上其他网络的默认路由；可一键还原 |
| 对端发现 | 两端都会在直连网段内广播自身信息，`peers` 列出对端、平台与可用空间；也可直接指定对端地址 |
| 游戏库扫描 | 解析 Steam 的 `libraryfolders.vdf` 与 `appmanifest_*.acf`，以及 Epic 的 `.item` 清单 |
| 分块传输 | 多连接并发拉取分块，每块附 `blake3` 摘要并逐块校验；临时共享的 SMB 路径作为兜底保留 |
| 断点续传 | 分块完成位图落盘，进程重启或断线后只补缺失分块，已完成部分不重传 |
| 差异传输 | 目标机已有旧版本时，用内容定义分块比对，只传变化的块 |
| 落盘认领 | 内容校验通过后原子改名到目标目录，再写入 `appmanifest_*.acf` 等清单文件 |
| 空间预检 | 传输前校验目标盘可用空间，不足时在开始前中止并报告缺口 |
| 桌面应用 | 三屏界面（扫描 → 确认 → 传输），进度与剩余时间实时刷新，全程不需要命令行 |
| 残留清理 | 不创建系统账号、不修改系统 ACL；SMB 兜底路径的临时共享随工具清理 |

## 仓库结构

```
crates/gamelift-core/       传输引擎：网口自举、对端发现、分块传输与校验、断点续传、差异比对
crates/gamelift-launchers/  平台适配器：Steam 库扫描、Epic 清单解析、VDF 解析、通用文件夹适配器
crates/gamelift-cli/        命令行客户端，依赖 core 与 launchers
apps/desktop/               Tauri 桌面应用：src-tauri 只写命令胶水层，src 是三屏界面
```

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
pnpm --dir apps/desktop tauri dev     # 本地起窗口
pnpm --dir apps/desktop tauri build   # 打安装包
```

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

不想用命令行时，启动桌面应用（`pnpm --dir apps/desktop tauri dev` 或安装包）：

1. 第一屏点「扫描游戏库」与「发现对端」，选一款游戏与一台对端机器。
2. 第二屏选角色：把本机游戏发给对端，或从对端接收。接收时会先检查目标盘空间，并对已有旧版本给出差异传输提示。
3. 第三屏看进度与剩余时间；作为源端时，把界面上的配对码与地址告诉对端即可。

## 状态

已实现命令行客户端与传输引擎：Steam 与 Epic 库扫描、直连自举、对端发现、多连接分块传输与逐块校验、断点续传、内容差异传输、空间预检与清单认领，SMB 与 robocopy 作为兜底路径保留；桌面应用提供三屏界面。后续计划：战网 / EA / Ubisoft / GOG 适配器、传输加密与限速。

## 参与贡献

- 开发规范：[AGENTS.md](./AGENTS.md)
- 贡献流程：[CONTRIBUTING.md](./CONTRIBUTING.md)
- 安全边界：[SECURITY.md](./SECURITY.md)

## 许可证

MIT，详见 [LICENSE](./LICENSE)。
