#!/usr/bin/env python3
"""Regenerates the .docx fixtures under crates/oco-docx/tests/data.

Two independent producers are used so the reader is not tuned to one of them:

  word-template.docx  python-docx, which starts from a template authored in
                      Word: theme fonts, the full built-in style set and
                      hand-injected tracked changes.
  libreoffice.docx    LibreOffice's own DOCX export of an HTML source.

Requires python-docx and a `soffice` binary on PATH.

    python3 tools/gen_fixtures.py
"""

import copy
import subprocess
import tempfile
from pathlib import Path

from docx import Document
from docx.oxml import parse_xml
from docx.oxml.ns import nsdecls, qn
from docx.shared import Pt, RGBColor

OUT = Path(__file__).resolve().parent.parent / "crates/oco-docx/tests/data"


def word_template() -> None:
    doc = Document()

    # Tell the theme which East Asian script the document uses, as a Chinese
    # edition of Word would. Without it `minorEastAsia` cannot be resolved.
    lang = doc.settings.element.find(qn("w:themeFontLang"))
    if lang is None:
        lang = parse_xml(f'<w:themeFontLang {nsdecls("w")} w:val="en-US"/>')
        doc.settings.element.append(lang)
    lang.set(qn("w:eastAsia"), "zh-CN")

    heading = doc.add_heading("第一章 项目概述", level=1)
    # "Strong" is bold and so is Heading 1: the two toggle each other off.
    heading.add_run("（非粗体）", style="Strong")

    p = doc.add_paragraph("OpencloudOffice 在浏览器中渲染文档。")
    p.add_run("粗体").bold = True
    p.add_run("斜体").italic = True
    red = p.add_run("红色大字")
    red.font.color.rgb = RGBColor(0xC0, 0x00, 0x00)
    red.font.size = Pt(16)

    # First-line indent of two characters, the usual Chinese body-text style.
    indented = doc.add_paragraph("首行缩进两个字符的正文段落。")
    indented.paragraph_format.element.get_or_add_pPr().append(
        parse_xml(f'<w:ind {nsdecls("w")} w:firstLineChars="200" w:firstLine="420"/>')
    )

    table = doc.add_table(rows=2, cols=2)
    for r in range(2):
        for c in range(2):
            table.cell(r, c).text = f"单元格 {r},{c}"

    # A paragraph with one tracked insertion and one tracked deletion.
    tracked = doc.add_paragraph("修订：保留")
    attrs = f'{nsdecls("w")} w:id="1" w:author="张三" w:date="2026-01-02T03:04:05Z"'
    tracked._p.append(parse_xml(f"<w:ins {attrs}><w:r><w:t>新增</w:t></w:r></w:ins>"))
    tracked._p.append(parse_xml(f"<w:del {attrs}><w:r><w:delText>删除</w:delText></w:r></w:del>"))

    doc.save(OUT / "word-template.docx")


HTML = """<!DOCTYPE html>
<html><head><meta charset="utf-8"></head><body>
<h1>LibreOffice 导出</h1>
<p>普通文字，<b>粗体</b>，<i>斜体</i>，<u>下划线</u>。</p>
<p style="text-align:center">居中段落</p>
<table border="1"><tr><td>甲</td><td>乙</td></tr><tr><td>丙</td><td>丁</td></tr></table>
</body></html>
"""


def libreoffice() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        source = Path(tmp) / "libreoffice.html"
        source.write_text(HTML, encoding="utf-8")
        subprocess.run(
            ["soffice", "--headless", "--convert-to", "docx:MS Word 2007 XML", "--outdir", tmp, str(source)],
            check=True,
            capture_output=True,
        )
        (OUT / "libreoffice.docx").write_bytes((Path(tmp) / "libreoffice.docx").read_bytes())


if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    word_template()
    libreoffice()
    print(f"wrote fixtures to {OUT}")
