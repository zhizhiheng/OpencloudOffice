# 0.0.1 图片来源

`opencloudoffice-0.0.1-hero.png` 使用内置 ImageGen 工具生成，是产品印象图。完整提示词保存在 [imagegen-prompt.txt](imagegen-prompt.txt)。生成后原样复制到项目中。

`viewer-document.png`、`viewer-search.png`、`viewer-footnotes.png` 是 Chromium 中实际运行查看器的截图，没有替换页面 DOM 或 CSS。截图使用随发行包提供的 DOCX 示例；其中嵌入的封面插图也是上述 ImageGen 图。

复现方式：

```sh
python3 tools/releases/make_demo.py
tools/web/build.sh
cargo build --release -p oco-cli
python3 tools/releases/package.py
OCO_WEB_DIR=out/releases/0.0.1/stage/opencloudoffice-0.0.1-web node tools/releases/capture.mjs
```

截图工具需要 Playwright 和 Chromium，可通过 `PLAYWRIGHT_MODULE`、`CHROMIUM_PATH` 指向已有安装。浏览器版本、窗口尺寸、缩放和查找结果见 [screenshots.json](screenshots.json)。
