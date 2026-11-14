# 参与开发

当前优先完善 DOCX 显示：读取格式、还原排版、图片、文字、标题、编号、页眉页脚和注释。编辑与写回安排在后续阶段。

`main` 是公开维护与发版主分支，版本使用 `v0.0.1` 形式的标签。功能开发使用独立分支，通过 Pull Request 说明问题、实现和验证结果。现有 Claude 和 Codex 分支保留。

核心能力优先自研，内核及 WASM 不增加第三方 crate。开发和测试工具可以沿用已有依赖。

修复显示问题时，请提供最小 DOCX、实际页面和预期页面，注明 Word / WPS 的版本及字体。区分规范推导、WPS 观察和 Word 实测。真实文件请先移除个人信息和业务秘密。

提交前运行与改动相关的检查：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
tools/web/build.sh
node tools/web/check.mjs
```

排版相关测试需要项目字体。查看器检查的安装和执行方法见 README。新增显示行为应有针对性的回归样例，并核对原生与 WASM；不能以总体分数增加掩盖某份文档退步。

提交贡献时，请确保你有权提交相关内容。贡献按本项目 Apache-2.0 许可证提供。
