# OpencloudOffice

自研的云端 Office 内核：在浏览器和 App 内渲染、编辑 Word、Excel、PDF 等文档，以 SDK 形式嵌入。

内核用 Rust 编写，编译成 WebAssembly 供浏览器使用，也可编译成原生库供 App 和服务端使用，各端排版结果一致。

**当前状态：研究原型阶段。** 已完成读取内核（读取 docx 并计算每段文字的最终格式，经 4725 个公开文件测试），以及排版引擎的原型（把正文、表格、页眉页脚和页码排成页面并输出 SVG，行高与 Word 实测一致）。表格按内容定列宽、合并单元格、跨页拆行都已实现；图片、脚注尚未排版，也还不能编辑。

还原度以 Word 为准并且可以度量：一条命令给出记分表，基线只许升不许降。对照 Word 365 导出的 128 份 PDF，目前 90.2% 的行断行相同、69.2% 的行位置在 0.5pt 以内；在引擎已覆盖的范围内（只含正文、字体齐全的 28 份）是 96.0% 和 87.6%，页数全部一致；34 份表格探针是 98.3% 和 95.7%。见 [docs/fidelity.md](docs/fidelity.md)。

| 文档 | 内容 |
|---|---|
| [docs/roadmap.md](docs/roadmap.md) | 目标、技术决定、原型计划和进度 |
| [docs/strategy.md](docs/strategy.md) | 如何做得比现有方案更好 |
| [docs/fidelity.md](docs/fidelity.md) | 还原度怎么量：标准答案的来源、当前记分表、只升不降的基线、接入 Word 输出 |
| [docs/layout.md](docs/layout.md) | 排版规则：行高、字号、缩进、编号、字体效果、标点，以及各自的依据 |
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

排版并输出页面（先运行 `tools/fonts/fetch.sh` 下载开源中文字体）：

```sh
cargo run --release -p oco-cli -- render testdata/zh/07-regulation.docx --out out/regulation
```

每页写成一个 SVG，字形以轮廓形式嵌入，用浏览器直接打开即可。

## 目录

| 路径 | 内容 |
|---|---|
| `crates/oco-docx` | docx 读取：容器、样式、主题字体、编号、页面设置、格式继承计算 |
| `crates/oco-layout` | 排版：字体度量与替换、断行、行高、编号、分页，输出 SVG |
| `crates/oco-cli` | 命令行工具 `oco`：`inspect` 查看格式，`render` 排版输出，`corpus` 批量测试，`score` 对照 Word 的分页记录打分 |
| `testdata/zh` | 11 份中文版式测试文档，由 `tools/zh/gen_corpus.py` 生成 |
| `testdata/zh-open` | 同样的 11 份，字体名换成开源字体，用于与 WPS 逐行对照；`wps/` 下是 WPS 的排版结果 |
| `tools/zh` | 生成中文测试文档，与 LibreOffice 的排版结果对比 |
| `tools/corpus` | 公开语料测试：拉取、与 LibreOffice 对比、与 Word 另存结果对照；生成带 Word PDF 的探针文档集 |
| `tools/fidelity` | 还原度记分表和基线 |
| `tools/word` | 用 Word 批量导出 PDF（Windows），并与我们的排版逐行对照 |
| `tools/wps` | 用 WPS 批量导出 PDF，作为第二参照 |
| `tools/fonts` | 下载开源字体；参照渲染用的字体映射 |
| `docs` | 见上表 |

## 开发

```sh
cargo test                                               # 全部测试
cargo clippy --all-targets                               # 静态检查
cargo build -p oco-layout --target wasm32-unknown-unknown  # 确认内核可编译成 WASM
tools/fidelity/score.py                                  # 还原度记分表，与基线比较（需先拉取测试集和字体）
```

排版测试需要字体：系统里的 Liberation、Carlito、DejaVu，以及 `tools/fonts/fetch.sh` 下载的中文字体。缺少字体时相关测试会打印提示并跳过。

语料测试的步骤见 [docs/corpus-report.md](docs/corpus-report.md) 的最后一节；排版对比的步骤见 `tools/zh/compare.py` 开头的说明。
