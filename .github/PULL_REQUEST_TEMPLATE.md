## 改动说明

<!-- 写"为什么"要这么改，而不是罗列改了什么文件。不要引用行号。 -->

## 自查清单

- [ ] `cargo fmt --all --check` / `cargo clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace` 全部通过
- [ ] 新逻辑有测试，且逻辑被破坏时测试会失败
- [ ] 未引入 GPL / AGPL / EUPL 许可证的依赖
- [ ] 涉及网络监听 / 文件写入的改动已对照 SECURITY.md 安全边界
- [ ] 重大决策已附 ADR
