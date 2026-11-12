# OpencloudOffice

自研的云端 Office 内核：在浏览器和 App 内渲染、编辑 Word、Excel、PDF 等文档，以 SDK 形式嵌入。

内核用 Rust 编写，编译成 WebAssembly 供浏览器使用，也可编译成原生库供 App 和服务端使用，各端排版结果一致。

**当前状态：研究原型阶段。** 目前完成了第一个原型（读取 docx 并计算每段文字的最终格式），并用 4725 个公开测试文件做了全面测试。还不能渲染或编辑。

| 文档 | 内容 |
|---|---|
| [docs/roadmap.md](docs/roadmap.md) | 目标、技术决定、原型计划和进度 |
| [docs/strategy.md](docs/strategy.md) | 如何做得比现有方案更好 |
| [docs/corpus-report.md](docs/corpus-report.md) | 语料测试的方法、数据和发现的问题 |
| [docs/word-behaviour.md](docs/word-behaviour.md) | Word 的实际行为与规范不同的地方 |

## 试用

需要 Rust 1.88 或更高版本。

```sh
cargo run -p oco-cli -- inspect crates/oco-docx/tests/data/word-template.docx
```

输出每个段落和每段文字在套用样式继承之后的最终格式，末尾列出文档中内核尚未支持的标记及出现次数：

```
#1 P style=Heading1 ("heading 1") outline=0 before=24pt after=0pt line=1.15x
    "第一章 项目概述"  font=Calibri / 宋体 14pt bold color=365F91
    "（非粗体）"  font=Calibri / 宋体 14pt color=365F91 char-style=Strong
...
Not modelled yet (16 kinds):
    649  w:style/w:tblStylePr
    100  w:style/w:tblPr
...
```

加 `--json` 输出结构化结果。

## 目录

| 路径 | 内容 |
|---|---|
| `crates/oco-docx` | docx 读取：zip 容器、样式表、主题字体、格式继承计算 |
| `crates/oco-cli` | 命令行工具 `oco`：`inspect` 查看单个文档，`corpus` 批量测试 |
| `tools/corpus` | 语料测试：拉取公开测试集、与 LibreOffice 对比、与 Word 另存结果对照 |
| `tools/gen_fixtures.py` | 重新生成单元测试用的 docx 文件 |
| `docs` | 见上表 |

## 开发

```sh
cargo test                                               # 全部测试
cargo clippy --all-targets                               # 静态检查
cargo build -p oco-docx --target wasm32-unknown-unknown  # 确认内核可编译成 WASM
```

语料测试的步骤见 [docs/corpus-report.md](docs/corpus-report.md) 的最后一节。
