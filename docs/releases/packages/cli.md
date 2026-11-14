# OpencloudOffice 0.0.1 · Linux CLI

用于文档检查、页面导出与自动化验证的原生命令行工具。本包面向 Linux x86_64，在 Ubuntu 22.04 上构建，使用 glibc。

## 检查版本与文档

```sh
./bin/oco --version
./bin/oco inspect samples/opencloudoffice-0.0.1.docx
./bin/oco inspect samples/opencloudoffice-0.0.1.docx --json
```

`inspect` 输出格式继承之后的段落、文字属性及未建模内容。

## 导出页面

本包不含字体。可使用浏览器发布包的 `fonts/` 目录，也可提供自己的 `.ttf`、`.otf` 或 `.ttc` 字体：

```sh
./bin/oco render samples/opencloudoffice-0.0.1.docx \
  --fonts /path/to/web-package/fonts \
  --out rendered
```

| 输出 | 内容 |
| --- | --- |
| `page-1.svg`、`page-2.svg`… | 每页自包含 SVG，字形以轮廓形式绘制。 |
| `lines.json` | 每页文字与行坐标。 |
| `layout.json` | 页数、缺失字体及文档范围摘要。 |

字体决定字形和排版。需要可重复的结果时，请固定字体文件版本。

## 进一步使用

`corpus` 用于批量读取与排版检查，`score` 对照 Word 在文档内保存的分页记录。完整用法见[开发指南](https://github.com/zhizhiheng/OpencloudOffice/blob/v0.0.1/docs/development.md)。

0.0.1 优先支持 DOCX 显示。DOC 二进制读取尚未实现；Markdown 当前仅转成普通段落；编辑和写回安排在后续阶段。

[项目首页](https://github.com/zhizhiheng/OpencloudOffice) · [发布说明](https://github.com/zhizhiheng/OpencloudOffice/releases/tag/v0.0.1) · [问题反馈](https://github.com/zhizhiheng/OpencloudOffice/issues)

源代码采用 Apache-2.0。命令行依赖的许可保存在 `licenses/crates/`，文件哈希及来源提交记录在 `manifest.json`。
