# OpencloudOffice 开发指南

本文面向从源码构建、集成或修改 OpencloudOffice 的开发者。直接体验产品可使用[浏览器试用包](https://github.com/zhizhiheng/OpencloudOffice/releases/tag/v0.0.1)。

## 环境准备

需要 Rust 1.88 或更高版本、Git；浏览器查看器还需要 `wasm32-unknown-unknown` 编译目标，以及用于启动静态服务的 Python 3。

```sh
git clone https://github.com/zhizhiheng/OpencloudOffice.git
cd OpencloudOffice
rustup target add wasm32-unknown-unknown
tools/fonts/fetch.sh
```

原生布局与部分测试使用项目字体，以及系统的 Liberation、Carlito、Caladea 和 DejaVu。Ubuntu 开发环境可安装：

```sh
sudo apt-get install fonts-liberation fonts-crosextra-carlito fonts-crosextra-caladea fonts-dejavu-core fonts-dejavu-extra
```

字体版本会影响换行和坐标。比较基准时应保留字体哈希；发布包使用 Liberation 2，并随包附上其余字体的许可和清单。

## 构建查看器

```sh
tools/web/build.sh
python3 -m http.server -d web 8000 --bind 127.0.0.1
```

打开 `http://localhost:8000`。构建脚本编译 WASM，并将字体及示例链接到 `web/`。字体解析、文档排版、查找和 SVG 生成由后台线程执行，页面按可见范围绘制。

查看器支持本地文件、拖入文件和文档网址，可通过 `?file=<网址>` 打开远程文档，也可添加本机字体。网址文件受浏览器跨域规则约束。本地文件只在浏览器里处理。

## 命令行工具

```sh
cargo build --release -p oco-cli
target/release/oco --version
target/release/oco inspect crates/oco-docx/tests/data/word-template.docx
target/release/oco inspect example.docx --json
target/release/oco render example.docx --fonts /path/to/fonts --out out/example
```

`inspect` 输出格式继承之后的段落与文字属性，以及未建模内容。`render` 输出每页 SVG、`lines.json` 和文档摘要。缺少字体时可运行 `tools/fonts/fetch.sh`，或用 `--fonts` 指定目录。

`corpus` 用于批量读取与布局检查；`score` 对照 Word 保存的分页记录。DOC 二进制读取尚未实现，Markdown 当前只转换为普通段落。

## JavaScript 接口

[`web/oco.js`](../web/oco.js) 提供同步 SDK。通过 `Oco.load` 加载 WASM、`addFont` 提供字体、`open` 打开文档，再调用 `pageSvg` 绘制页面；使用完后关闭文档句柄。

[`viewer-client.js`](../web/viewer-client.js) 与 [`viewer-worker.js`](../web/viewer-worker.js) 提供后台线程接口。完整使用方式可参考 [`viewer.js`](../web/viewer.js)。

## 验证改动

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release -p oco-cli
tools/web/build.sh
node tools/web/check.mjs testdata/zh/*.docx testdata/probes/*.docx testdata/release/*.docx
```

缺少字体时部分测试会提示并跳过；完整排版验证需要准备字体。原生与 WASM 检查逐页比较 SVG，打开失败会报错。

查看器集成检查需要 Playwright 和 Chromium：

```sh
npm install --no-save playwright
npx playwright install chromium
node tools/web/test-viewer.mjs
node tools/web/test-display-details.mjs
```

也可通过 `PLAYWRIGHT_MODULE`、`CHROMIUM_PATH` 指向已有安装。检查包括查找、跳页、缩放、复制、后台处理、文件切换、下载失败和字体添加。

逐文档还原度检查使用 `tools/fidelity/score.py`，需要先准备语料与参照。方法见[还原度记录](fidelity.md)。新增规则应有最小样例，并分别注明 Word 实测、WPS 观察或规范推导。

## 项目目录

| 路径 | 用途 |
| --- | --- |
| `crates/oco-docx` | DOCX 容器、ZIP / DEFLATE、XML、结构、样式、主题与编号读取。 |
| `crates/oco-layout` | 字体解析、字形、断行、分页、表格、图片、注释和 SVG 输出。 |
| `crates/oco-wasm` | 手写 WebAssembly 接口。 |
| `crates/oco-cli` | 格式检查、批量处理、页面导出和对照工具。 |
| `crates/oco-formats` | 格式识别与统一入口。 |
| `crates/oco-doc` / `oco-markdown` | DOC 和 Markdown 的转换入口，当前实现范围见发布说明。 |
| `web` | 浏览器查看器、同步 SDK 与后台线程接口。 |
| `testdata` | 中文样例、规则探针和发布示例。 |
| `tools/word` / `wps` / `fidelity` | 参考导出、逐行比较和逐文档基线。 |
| `tools/releases` | 发布示例生成、打包、截图与发行包检查。 |

## 生成发布材料

`tools/releases/package.py` 在 Ubuntu 环境收集发行包所需字体和许可，需要额外安装 `fonts-liberation2`。主视觉保存在 `docs/images/opencloudoffice-hero.jpg`。

```sh
python3 tools/releases/make_demo.py
cargo build --release -p oco-cli
tools/web/build.sh
python3 tools/releases/package.py
node tools/releases/verify.mjs
OCO_WEB_DIR=out/releases/0.0.1/stage/opencloudoffice-0.0.1-web node tools/releases/capture.mjs
```

输出位于 `out/releases/0.0.1/`；截图写入 `docs/images/`。打包清单记录来源提交、字体来源、文件大小与 SHA-256。发布前确认清单提交与版本标签一致。
