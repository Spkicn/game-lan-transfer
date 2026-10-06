# 贡献指南

感谢参与 GameLift！

## 环境要求

| 工具 | 版本 | 说明 |
|---|---|---|
| Rust | 见 `rust-toolchain.toml` | rustup 自动安装 |
| Windows | 10 1809+ / 11 | 当前仅支持 Windows 平台 |

## 快速开始

```bash
git clone https://github.com/Spkicn/game-lan-transfer.git
cd game-lan-transfer
cargo test --workspace                    # 应全部通过
cargo run -p gamelift-cli -- scan         # 扫描本机游戏库
```

## 开发规范（必读）

- **[AGENTS.md](./AGENTS.md)**：代码规范、注释规范、命令与提交规范。
- **[SECURITY.md](./SECURITY.md)**：安全红线。本项目涉及网络监听与文件写入，改动前必读。
- 重大技术决策先写 ADR 再编码，记录背景、决策、备选方案与后果。

## 提交流程

1. 从 `main` 切分支：`<username>/<scope>/<short-desc>`
2. 开发 + 自测，提交前检查全部通过：
   ```bash
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
3. PR 描述写"为什么"这么改，不引用行号
4. CI（fmt / clippy / test）全绿后 squash merge

## 报告问题

提 issue 请选择对应模板（bug 报告必须带版本号与日志）。安全问题请勿开公开 issue，见 [SECURITY.md](./SECURITY.md)。

## 发版

1. 版本号有**三处必须同时改**，漏一处会导致安装包名与标签对不上：
   - `Cargo.toml` 的 `[workspace.package] version`
   - `apps/desktop/src-tauri/tauri.conf.json` 的 `version`
   - `apps/desktop/package.json` 的 `version`
2. 版本号按语义化版本升：只修缺陷升 patch，新增能力升 minor，破坏兼容升 major
3. 提交后打标签 `app-vX.Y.Z` 并推送，标签触发 `.github/workflows/release.yml`，产物先落成草稿发布
4. 检查草稿里的安装包资产，正文按固定顺序写：先放安装包 SHA-256 表（下载页第一件事是核对），再放分类变更清单、带链接的下载表与已知限制
5. 未做代码签名、未在真机验证过的版本一律勾选预发布
6. 工作流失败先判断是网络还是凭据问题，再重跑同一条工作流；不要手工补资产，手工产物与工作流产物不一致会让下一次发布对不上
