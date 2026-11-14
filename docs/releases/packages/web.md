# OpencloudOffice 0.0.1

![OpencloudOffice](assets/hero.jpg)

**浏览器查看器 · 开源开发者预览**

打开 DOCX，在浏览器本地完成格式读取、排版和绘制。本包包含运行所需的 WASM、37 个开源字体文件与示例文档，无需安装 Rust。

## 三步开始

1. 解压此发布包。
2. 在当前目录启动 HTTP 服务。
3. 打开 **http://localhost:8000**，选择示例文档或拖入自己的 DOCX。

```sh
python3 -m http.server 8000 --bind 127.0.0.1
```

也可使用其他静态 HTTP 服务。请通过 HTTP 访问，直接用 `file://` 打开无法加载 Worker 和 WASM。

## 阅读一份文档

| 操作 | 使用方式 |
| --- | --- |
| 打开文件 | 点击“打开文件”，或将 DOCX 拖到窗口。 |
| 查看发布示例 | 在“示例文档”中选择 `opencloudoffice-0.0.1.docx`。 |
| 查找内容 | 点击“查找”或按 Ctrl/Cmd+F，搜索整份文档。 |
| 跳转页面 | 在页码框输入数字并按回车。 |
| 调整显示 | 缩放、适合宽度、显示文档信息，或添加本机字体。 |

本地文件不会上传。网址打开功能会访问你输入的文档地址，并受浏览器跨域规则约束。

## 包内文件

| 路径 | 内容 |
| --- | --- |
| `index.html` | 查看器入口。 |
| `oco.wasm` / `oco.js` | 显示内核与 JavaScript 接口。 |
| `viewer-*.js` / `viewer.js` | 后台线程接口和查看器。 |
| `fonts/` | 开源字体与加载索引。 |
| `samples/` | 发布手册和版式示例。 |
| `manifest.json` | 来源提交、字体来源、文件大小和 SHA-256。 |
| `licenses/` | 字体许可原文。 |

## 当前范围

0.0.1 优先支持 DOCX 显示。编辑与写回、DOC 二进制读取、完整 Markdown，以及更多 Office 格式安排在后续阶段。复杂版式、浮动对象、图片旋转和裁切仍需完善。

[项目首页](https://github.com/zhizhiheng/OpencloudOffice) · [发布说明](https://github.com/zhizhiheng/OpencloudOffice/releases/tag/v0.0.1) · [问题反馈](https://github.com/zhizhiheng/OpencloudOffice/issues)

源代码采用 Apache-2.0；第三方字体保留原许可。详见 `LICENSE`、`NOTICE` 和 `THIRD_PARTY_NOTICES.md`。
