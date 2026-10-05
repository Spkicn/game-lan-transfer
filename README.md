# GameLift

GameLift 是一个 Windows 局域网游戏迁移工具。它把一台机器上已经安装好的游戏，经点对点局域网连接迁移到另一台机器，并在迁移完成后写入平台清单文件，使目标机的启动器将游戏识别为已安装，无需重新下载。

工具面向"两台机器之间存在高速链路、但互联网出口带宽有限"的场景。传输过程只读取源端内容，不修改源端的任何文件。

## 功能

| 模块 | 说明 |
|---|---|
| 直连自举 | 识别物理以太网口并配置点对点静态地址，不设置网关，不改变机器上其他网络的默认路由；可一键还原 |
| 游戏库扫描 | 解析 Steam 的 `libraryfolders.vdf` 与 `appmanifest_*.acf`，输出游戏名称、体积与版本指纹 |
| 内容传输 | 源端建立临时只读 SMB 共享，目标端用 robocopy 拉取，支持断点续传与多线程 |
| 落盘认领 | 复制 `appmanifest_*.acf` 并给出启动器认领步骤，避免目标机重复下载 |
| 空间预检 | 传输前校验目标盘可用空间，不足时在开始前中止并报告缺口 |
| 残留清理 | 临时共享随工具清理；不创建系统账号，不修改系统 ACL |

## 仓库结构

```
crates/gamelift-core/       传输引擎：网口自举、空间查询、临时共享、robocopy 拉取、清单落盘
crates/gamelift-launchers/  平台适配器：Steam 库扫描、VDF 解析、通用文件夹适配器
crates/gamelift-cli/        命令行客户端，依赖 core 与 launchers
```

## 构建

要求 Windows 10 1809+ 或 Windows 11，Rust 工具链版本见 `rust-toolchain.toml`。

```bash
cargo build --release -p gamelift-cli
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## 使用

```text
gamelift scan                 扫描本机 Steam 库
gamelift nics                 列出网口，排查直连问题
gamelift link --host 1|2      配置直连地址，需要管理员权限
gamelift unlink               还原直连配置
gamelift serve <appid>        源端：共享指定游戏，需要管理员权限
gamelift stop                 源端：删除共享
gamelift pull <appid> --peer <IP> [--drive D] [--dest <路径>]
                              目标端：拉取游戏并给出认领指引
```

一次完整迁移：

1. 两台机器分别执行 `gamelift link`（一端 `--host 1`，另一端 `--host 2`）。
2. 源端执行 `gamelift scan` 找到目标游戏的 `appid`，再执行 `gamelift serve <appid>`。
3. 目标端执行 `gamelift pull <appid> --peer <源端地址>`。`--drive` 指定用于空间预检的目标盘，`--dest` 在目标机未安装该游戏时显式指定目标目录。
4. 按 `pull` 输出的收尾指引重启启动器并验证文件完整性；源端执行 `gamelift stop` 清理共享。

## 状态

已实现命令行客户端与传输引擎，覆盖 Steam 库扫描、直连自举、SMB 与 robocopy 传输以及清单文件认领。后续计划：桌面应用、内容差异传输、对端自动发现与配对、Steam 之外的平台适配器。

## 参与贡献

- 开发规范：[AGENTS.md](./AGENTS.md)
- 贡献流程：[CONTRIBUTING.md](./CONTRIBUTING.md)
- 安全边界：[SECURITY.md](./SECURITY.md)

## 许可证

MIT，详见 [LICENSE](./LICENSE)。
