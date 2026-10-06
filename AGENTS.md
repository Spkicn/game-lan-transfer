# 开发规范

本文件是 GameLift 的开发规范，适用于所有贡献者。贡献流程见 [CONTRIBUTING.md](./CONTRIBUTING.md)，安全边界见 [SECURITY.md](./SECURITY.md)。

## 1. 项目概览

GameLift：Windows 局域网游戏迁移工具。两台机器经网线或局域网互联，把源端已安装的游戏迁移到目标端，并写入平台清单文件，使目标机的启动器直接识别为已安装。

**技术栈**：Rust（核心逻辑，workspace 多 crate）+ Tauri v2 + TypeScript（桌面壳）。

**目录地图**：

```
crates/gamelift-core/      传输引擎：网口自举 / 传输 / 校验 / 进度，零 GUI 依赖
crates/gamelift-launchers/ 平台适配器：Steam / Epic / 战网 / EA / Ubisoft / GOG 的库扫描与清单写入
crates/gamelift-cli/       命令行客户端，依赖 core + launchers
apps/desktop/              Tauri 桌面应用：src-tauri/ 只写 #[tauri::command] 胶水层
```

**心智模型**：文件系统、网络、进程操作全部在 Rust 侧完成；前端只做 UI 与状态，经 Tauri command / event 通信，禁止在前端做任何 IO。

## 2. 常用命令

**改完代码后的提交前检查，全部通过才算完成：**

```bash
# Rust，在仓库根目录执行
cargo fmt --all --check                                  # 格式检查
cargo clippy --workspace --all-targets -- -D warnings    # lint，warning 即失败
cargo test --workspace                                   # 全部测试
cargo run -p gamelift-cli -- scan                        # 试运行 CLI

# 桌面应用，在 apps/desktop 下执行
pnpm typecheck                                           # tsc --noEmit
pnpm test                                                # 前端单元测试
pnpm build                                               # 前端产物
```

**Windows 注意**：开发环境是 Git Bash + PowerShell；路径含空格要加引号。测试输出写到 `target/tmp/`，不要用系统盘临时目录。

## 3. 代码规范 — Rust

- **禁止 `unwrap()` / `expect()`** 出现在非测试代码。例外：锁中毒（`.expect("lock poisoned")`）与测试。
- 错误处理分层：库 crate（core / launchers）用 `thiserror` 定义错误类型；应用壳（src-tauri / cli）用 `anyhow`。禁止用 `String` 当错误类型跨 crate 边界。
- 避免不必要的 `.clone()`，能借用就借用。
- async 规则：不嵌套创建 runtime；不跨 `.await` 持锁；阻塞 IO 用 `spawn_blocking`；async 代码里禁用 `std::thread::sleep`。
- 新增依赖必须在 PR 描述里说明理由；许可证必须是 MIT / Apache-2.0 / BSD-3 / MPL-2.0 之一（项目要求，闭源友好），GPL / AGPL / EUPL **禁止**。
- 改 `gamelift-core` 的公共 API 前自问："这个改动会不会破坏依赖它的代码？"签名能不动就不动。
- 高频路径（传输循环、分块回调）禁止 `debug!` 及以上日志，用 `trace!` 或节流。

## 4. 代码规范 — TypeScript

桌面应用（`apps/desktop`）适用：

- `strict: true`；禁止 `any`（确实需要时用 `unknown` + 收窄并注释理由）。
- 前端不 import Node API，只通过 `@tauri-apps/api` 与后端通信。
- Tauri command 的参数和返回值必须有显式类型；改动 `capabilities/` 权限文件必须人工确认。
- 事件命名：`transfer://<事件名>`（如 `transfer://progress`）。

## 5. 注释规范

- **只说"干什么"**：一行为主，最多两行；禁止复述代码一眼可见的内容。
- **禁止括号补充**：不用 `（...）` 附带说明，内容写进正文或删除。
- **禁止结尾句号**：描述句末不加 `。`，`# Errors` 等小节条目同样不加。
- **禁止罗列主逻辑**：不用 `-` / `*` / `1. 2.` 罗列步骤；只有异常场景与边界条件允许列举。
- **禁止空洞词汇**：承担、旨在、解耦、核心方案、顾名思义等一律不用。
- 公开项必须有文档注释：Rust 用 `///`，crate 与模块用 `//!`；`# Errors` 只写异常场景，由 clippy 要求的地方不得删除。
- 中文优先，专业术语保留英文（SMB、robocopy、buildid 等）。
- 修改逻辑必须同步更新注释，过期注释视为缺陷。

## 6. 测试要求

- 新逻辑必须有测试，且**测试在逻辑被破坏时必须失败**（改实现前先确认测试会红）。
- Rust 单元测试放模块内 `#[cfg(test)] mod tests`；集成测试放各 crate 的 `tests/` 目录。
- 网络相关测试只绑 `127.0.0.1`，端口从 `18000+` 顺次分配，不依赖外网。
- 按改动风险选择验证：改一行文案不必全量构建；动协议/传输层必须跑 `cargo test -p gamelift-core` 全量。

## 7. Git 与提交规范

- **Conventional Commits，scope 必填**：`<type>(<scope>): <subject>`。
  - type：`feat` `fix` `chore` `docs` `style` `refactor` `perf` `test` `build` `ci`
  - scope：`core` `launchers` `cli` `ui` `shell` `proto` `ci` `deps` `docs`
  - 示例：`feat(launchers): 解析 Epic .item 清单`；破坏性变更加 `!` 并在 body 写 `BREAKING CHANGE:`。
- subject 中文、首词动词、≤50 字；正文写"为什么"而不是"改了什么"。
- 不 force push；PR 用 squash merge；PR 描述不引用行号（会过期）。
- 分支命名：`<username>/<scope>/<short-desc>`，如 `alice/launchers/epic-manifest`。
- 不添加 `Co-Authored-By` 或其他署名 trailer；提交信息使用与 GitHub 账号一致的作者身份。

## 8. 行为守则

- **最小 diff**：只改完成任务所需的内容。不做顺带重构，不动无关代码，不删既有注释。
- 修改他人已改过的文件前先重新读取，保留其编辑和注释。
- 完成前跑提交前检查；失败就修，不要声称完成。
- 不引用行号描述代码；依赖版本、crate 现状查 `Cargo.toml` / `Cargo.lock`，不凭记忆。
- 自建的临时文件、脚本、调试代码，完工前清理。
- 收尾时指出你可能没考虑到的问题（如磁盘空间、边界条件、许可证）。

**安全红线（无例外）**：
- 只监听局域网接口，任何监听代码改动都要确认不绑定 `0.0.0.0` 到公网可达场景。
- 接收端不信任发送方的文件名与路径，必须做路径穿越净化。
- 不 `eval` 任何外部输入；不修改源端（发送方）的任何文件。

## 9. 架构决策

重大技术决策必须先写 ADR 再动代码，记录背景、决策、备选方案与后果。

## 10. 发版与真机验证

**真机验证通过之前不发版。** 这条优先于任何图省事的发版习惯。

流程固定四步：

1. 改完代码，跑完第 2 节的提交前检查；
2. 在本地构建安装包：`pnpm --dir apps/desktop tauri build`，产物在 `target/release/bundle/nsis/`；
3. 把安装包交给使用者在真实的两台机器上验证，等验证结果；
4. 只有使用者确认没问题，才打标签发版。

配套约定：

- **未验证的构建不进 Release，也不打标签**；标签只在第 4 步创建
- 版本号可以提前写进三处配置，但发布动作必须等验证通过
- 验证不通过就改代码、重新构建安装包，重复第 2 到第 3 步；这个过程不产生任何发布
- 交付安装包时同时给出产物路径与 SHA-256，使用者拿到的是同一个文件
- 真机验证结果由使用者提供，不代写、不推测；没验证过的项在发布说明里如实写明

发版后才发现问题时：

- 资产下载次数为 0：撤版并复用同一个版本号重发，撤版要连标签一起撤
- 资产下载次数不为 0：升一版，保留旧版
- 同一处改动不连发两版。版本号是给使用者的，不是开发者的时间线
