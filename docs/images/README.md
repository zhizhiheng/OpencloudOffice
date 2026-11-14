# 0.0.1 图片来源

`opencloudoffice-hero.jpg` 是项目作者提供的产品主视觉，原图尺寸为 1280 × 720，按原始 JPEG 文件保存，没有裁切或重新编码。

`preview-document.png`、`preview-search.png`、`preview-footnotes.png` 是 Chromium 中实际运行查看器的截图，没有替换页面 DOM 或 CSS。截图使用随发行包提供的 DOCX 示例，示例封面使用同一张产品主视觉。

复现方式：

```sh
python3 tools/releases/make_demo.py
tools/web/build.sh
cargo build --release -p oco-cli
python3 tools/releases/package.py
OCO_WEB_DIR=out/releases/0.0.1/stage/opencloudoffice-0.0.1-web node tools/releases/capture.mjs
```

截图工具需要 Playwright 和 Chromium，可通过 `PLAYWRIGHT_MODULE`、`CHROMIUM_PATH` 指向已有安装。浏览器版本、窗口尺寸、缩放和查找结果见 [screenshots.json](screenshots.json)。
