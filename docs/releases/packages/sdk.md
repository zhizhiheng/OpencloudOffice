# OpencloudOffice 0.0.1 · WASM SDK

将自研 DOCX 显示内核接入网页或应用。包内提供 WebAssembly 模块、同步 JavaScript SDK，以及查看器使用的后台线程接口。

## 接入准备

通过 HTTP 服务提供这些文件，并准备文档需要的 `.ttf`、`.otf` 或 `.ttc` 字体。本 SDK 包不含字体；可从完整浏览器发布包取得开源字体，并保留对应许可。

## 绘制第一页

页面需要一个文件输入框和承载页面的元素：

```html
<input id="file" type="file" accept=".docx" />
<div id="page"></div>
```

在用户选中文件后，加载字体和文档：

```js
import { Oco } from "./oco.js";

const core = await Oco.load("./oco.wasm");
const font = await fetch("./fonts/SourceHanSerifCN-Regular.otf");
core.addFont(await font.arrayBuffer());

document.querySelector("#file").addEventListener("change", async (event) => {
  const file = event.target.files[0];
  if (!file) return;
  const doc = core.open(await file.arrayBuffer(), file.name);
  try {
    document.querySelector("#page").innerHTML = doc.pageSvg(0);
    console.log(doc.info());
  } finally {
    doc.close();
  }
});
```

## 包内接口

| 文件 | 用途 |
| --- | --- |
| `oco.wasm` | 默认不依赖第三方 crate 的显示内核。 |
| `oco.js` | 同步加载字体、打开文档、绘制页面及读取文档信息。 |
| `viewer-client.js` / `viewer-worker.js` | 后台线程接口；完整调用方式见项目中的 `viewer.js`。 |
| `manifest.json` | 来源提交和文件 SHA-256。 |

这是首个开发者预览版本，接口会随后续版本演进。当前重点是 DOCX 显示，编辑和写回尚未提供。

[开发指南](https://github.com/zhizhiheng/OpencloudOffice/blob/v0.0.1/docs/development.md) · [查看器源码](https://github.com/zhizhiheng/OpencloudOffice/tree/v0.0.1/web) · [问题反馈](https://github.com/zhizhiheng/OpencloudOffice/issues)

源代码采用 Apache-2.0，详见包内 `LICENSE` 和 `NOTICE`。
