# 单条规则的探针

这里的每份文档只考几条排版规则，用来让参照软件排一遍、量出规则，再和我们的引擎逐行对照。

- 文档由 `tools/probes/make.py` 生成，每次生成的字节相同；西文用 Liberation Serif，中文用思源宋体（`fonts/` 里的开源字体），参照机器上都有，不会发生字体替换。
- `wps/` 下是 WPS Office for Linux 排出的结果：每份文档一个 `*.lines.json`，记着每一行的文字和位置（PDF 太大，没有入库）。重新导出：`python3 tools/wps/export.py testdata/probes`。
- `tools/fidelity/score.py` 把它们单列一栏计分，并记入只升不降的基线。

| 文档 | 考什么 |
|---|---|
| `paragraph-borders.docx` | 段落四周的边框和底纹：线和填充的位置、占多少地方、相邻段落合成一个框、带缩进时的边框 |
| `shading-and-tables.docx` | 段落底纹怎样穿过段前段后间距；三张表格：单元格自己的线与表格的线、虚线和没有线的内部线、完全没有线、单元格底纹 |
| `table-lines.docx` | 同上，表格另外写明了单元格的左右边距 |
| `notes-continuous.docx` | 脚注在页面底部的位置；全文连续编号 |
| `notes-each-page.docx` | 每页重新编号 |
| `notes-each-section.docx` | 两节，每节重新编号；第二节用大写罗马数字，从 V 起 |
| `notes-section-start.docx` | 两节，连续编号；第二节写了起始号 5 |
| `grid-foot-25-0.docx`、`grid-foot-25-5.docx` | 文档网格下页底最后一行需要多少地方：30 行中文，每行占两格；下边距给第 22 行留 25.0 磅和 25.5 磅，WPS 只在后一份里把它留在第一页 |

规则本身和量出来的数值记在 [排版研究](../../docs/layout.md)，怎样计分记在 [还原度](../../docs/fidelity.md)。

WPS 不是标准：它与 Word 有差别时以 Word 为准。目前已知并保留的差别：粗边线的单元格里，Word 把文字往里多让半条线，WPS 不让（两份表格探针各有 2 行因此位置不同）。
