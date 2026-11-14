# 第三方材料

OpencloudOffice 源代码采用 Apache-2.0。第三方字体、测试材料和开发依赖保留各自许可；项目许可证不替代它们的许可。

## 产品内核

默认配置的 DOCX 读取、ZIP / DEFLATE、XML、字体文件解析、排版和 WASM 内核不依赖第三方 crate。命令行工具启用可选的 serde 功能，用于输出开发报告。

命令行发布包包含实际依赖的许可证原文及清单。依赖的名称和版本以 `Cargo.lock` 为准，许可证与源代码来自对应的 crates.io 包。测试用的浏览器自动化、PDF 读取器、Word 和 WPS 不包含在产品发布包中。

## 浏览器发布包中的字体

| 字体 | 来源 | 字体许可 |
| --- | --- | --- |
| Source Han Sans CN | [Adobe Source Han Sans](https://github.com/adobe-fonts/source-han-sans) | SIL Open Font License 1.1 |
| Source Han Serif CN | [Adobe Source Han Serif](https://github.com/adobe-fonts/source-han-serif) | SIL Open Font License 1.1 |
| LXGW WenKai | [霞鹜文楷](https://github.com/lxgw/LxgwWenKai) | SIL Open Font License 1.1 |
| Liberation 2 | [Liberation Fonts](https://github.com/liberationfonts/liberation-fonts) | SIL Open Font License 1.1 |
| Carlito | [Google Fonts / Carlito](https://github.com/googlefonts/carlito) | SIL Open Font License 1.1 |
| Caladea | 系统 `fonts-crosextra-caladea` 包所提供的版本 | Apache-2.0 |
| DejaVu Sans / Serif / Mono | [DejaVu Fonts](https://dejavu-fonts.github.io/) | Bitstream Vera 许可；DejaVu 的修改为公有领域 |

发行包只包含上述字体的未修改文件，并附原始声明。Liberation 使用 2.x 的 OFL 版本，不打包开发机可能安装的旧 1.x 字体。字体文件、原始来源和 SHA-256 记录在发行包的 `manifest.json` 中。字体版本会影响排版，仓库中历史对照分数使用各自记录的字体环境。

源码仓库不存放第三方字体二进制。`tools/fonts/fetch.sh` 按固定提交获取开源中文字体；发布工具再从已安装字体中收集明确列出的字体。

## 测试材料与图片

- `testdata/zh`、`testdata/zh-open`、`testdata/probes`、`testdata/details` 和 `testdata/release` 的生成器随仓库提供。
- Word 对照材料来自 [docx4j](https://github.com/plutext/docx4j)，对应提交和来源记录在审计文档中。大语料及参考 PDF 不包含在浏览器或命令行发行包中。
- `docs/images/preview-*.png` 是 OpencloudOffice 的真实运行截图。`opencloudoffice-hero.jpg` 是项目作者提供的产品主视觉，按原图保存。
- Microsoft Word 和 WPS Office 的名称仅用于说明对照来源；它们不属于本项目。
