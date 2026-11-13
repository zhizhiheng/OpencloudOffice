# OpencloudOffice

自研的云端 Office 内核：在浏览器和 App 内渲染、编辑 Word、Excel、PDF 等文档，以 SDK 形式嵌入。当前阶段只做一件事：把文档预览做到极致，范围是 DOCX、DOC 和 Markdown。

内核用 Rust 编写，编译成 WebAssembly 供浏览器使用，也可编译成原生库供 App 和服务端使用，各端排版结果一致。

**当前状态：研究原型阶段。** 已完成读取内核（读取 docx 并计算每段文字的最终格式，经 4725 个公开文件测试），以及排版引擎的原型（把正文、表格、页眉页脚、页码、图片和文本框排成页面并输出 SVG，行高与 Word 实测一致）。内核已经能在浏览器里跑：`web/` 下有一个查看器，打开本地或网址上的 docx，逐页显示。脚注、分栏尚未排版，也还不能编辑。

**原生实现。** zip 解压、XML 解析、字体文件解析（TrueType 和 CFF 轮廓、字符映射、度量）都是自己写的，约 3500 行；内核不依赖任何第三方库，只有开发用的命令行工具为了输出 JSON 报告用了 serde。浏览器接口是手写的，不用 wasm-bindgen 之类的生成工具。字体解析替换前与原来的库逐字形对比过：316 个字体文件、147 万个字形，轮廓、字宽、度量没有一处不同。换掉通用 zip 库之后，4 份带重名条目的探针文档读得和 Word 一样了（通用库取后一个条目，Word 取目录里的前一个）。

还原度以 Word 为准并且可以度量：一条命令给出记分表，基线只许升不许降。对照 Word 365 导出的 128 份 PDF，目前 92.9% 的行断行相同、78.2% 的行位置在 0.5pt 以内；在引擎已覆盖的范围内（只含正文、字体齐全的 28 份）是 96.0% 和 87.6%，页数全部一致；34 份表格探针是 98.3% 和 95.7%。见 [docs/fidelity.md](docs/fidelity.md)。

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

在浏览器里看（需要 `rustup target add wasm32-unknown-unknown`）：

```sh
tools/web/build.sh                     # 编译 web/oco.wasm，并把字体和示例文档链接到 web/ 下
python3 -m http.server -d web 8000     # 然后打开 http://localhost:8000
```

查看器可以打开本地文件（也可以拖进来）、网址上的文件（`?file=<网址>`），可以添加本机的字体文件；侧栏列出这份文档里还不能按 Word 的样子显示的内容。文件只在浏览器里读取和排版，不上传。

在页码框输入数字后按回车可以跳页。点击“查找”或按 Ctrl/Cmd+F 可搜索整份文档，包括尚未滚动到的页面；匹配处会高亮，回车跳到下一个，Shift+回车跳到上一个，Esc 关闭查找。结果超过 1000 处时显示前 1000 处，并标记“1000+”。文字仍可选中、复制。缩放和切换侧栏会尽量保持当前阅读位置。

字体解析、文档排版、查找和 SVG 生成在 Web Worker 中执行，页面按可见范围逐页绘制。字体按原有顺序分批下载并解析；当前仍加载完整字体集，按文档按需下载尚未实现。快速切换文件时以最后选择的文件为准，打开失败会保留原文档。

`web/oco.js` 是同步 JavaScript SDK；`web/viewer-client.js` 和 `web/viewer-worker.js` 为查看器提供异步接口。

浏览器里排出的页面与命令行逐字节相同：`node tools/web/check.mjs` 对 259 份文档、658 页做过比较，没有一处不同；该脚本会对打开失败的文件报错。构建脚本会显示当前 WASM 的大小及 gzip 后大小，字体文件另计。

## 目录

| 路径 | 内容 |
|---|---|
| `crates/oco-docx` | docx 读取：zip 和 XML 的读取、容器、样式、主题字体、编号、页面设置、图片、格式继承计算 |
| `crates/oco-doc` | 二进制 Word 格式（.doc）转成文档形式（进行中） |
| `crates/oco-markdown` | Markdown 转成文档形式（进行中） |
| `crates/oco-formats` | 统一入口：判断文件是哪一种，读成文档形式 |
| `crates/oco-layout` | 排版：字体文件解析、字体度量与替换、断行、行高、编号、表格、图片、分页，输出 SVG |
| `crates/oco-wasm` | 内核的 WebAssembly 接口：一组 C 接口函数，不引入任何外部符号 |
| `web` | 浏览器查看器，以及给 JavaScript 用的接口 `oco.js` |
| `tools/web` | 构建查看器；检查 WASM 版与原生版输出一致 |
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

查看器的浏览器集成检查：先运行 `tools/web/build.sh`，在开发环境安装 Playwright 及其 Chromium，再运行 `node tools/web/test-viewer.mjs`。也可用 `PLAYWRIGHT_MODULE`、`CHROMIUM_PATH` 指向现有安装。检查包括全文搜索、跳页、缩放位置、后台处理时的界面响应、快速切换文件、下载失败和复制；截图写入 `out/viewer-tests/`。

排版测试需要字体：系统里的 Liberation、Carlito、DejaVu，以及 `tools/fonts/fetch.sh` 下载的中文字体。缺少字体时相关测试会打印提示并跳过。

语料测试的步骤见 [docs/corpus-report.md](docs/corpus-report.md) 的最后一节；排版对比的步骤见 `tools/zh/compare.py` 开头的说明。
