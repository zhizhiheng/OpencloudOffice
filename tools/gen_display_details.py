#!/usr/bin/env python3
"""Build small, reproducible display probes with the Python standard library.

These are test inputs, not Word/WPS reference renderings. Runtime parsing,
colour conversion, fields, layout and SVG emission remain in our Rust core.
"""
from pathlib import Path
import struct
import zlib
import zipfile
from itertools import count
from xml.sax.saxutils import escape

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "testdata/details"
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
A = "http://schemas.openxmlformats.org/drawingml/2006/main"
WP = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
PIC = "http://schemas.openxmlformats.org/drawingml/2006/picture"
NS = f'xmlns:w="{W}" xmlns:r="{R}" xmlns:a="{A}" xmlns:wp="{WP}" xmlns:pic="{PIC}"'
PICTURE_IDS = count(1)


def run(text, props=""):
    return f'<w:r><w:rPr>{props}</w:rPr><w:t xml:space="preserve">{escape(text)}</w:t></w:r>'


def paragraph(content, props=""):
    return f"<w:p><w:pPr>{props}</w:pPr>{content}</w:p>"


def field(code):
    return f'<w:fldSimple w:instr="{escape(code)}">{run("99")}</w:fldSimple>'


def picture(flip="", crop="", width=100, height=60):
    identity = next(PICTURE_IDS)
    return f'''<w:r><w:drawing><wp:inline><wp:extent cx="{width * 12700}" cy="{height * 12700}"/>
      <wp:docPr id="{identity}" name="four-colour-probe"/><a:graphic><a:graphicData uri="{PIC}"><pic:pic>
      <pic:nvPicPr><pic:cNvPr id="{identity}" name="quadrants.png"/><pic:cNvPicPr/></pic:nvPicPr>
      <pic:blipFill><a:blip r:embed="image"/>{f'<a:srcRect {crop}/>' if crop else ''}<a:stretch><a:fillRect/></a:stretch></pic:blipFill>
      <pic:spPr><a:xfrm {flip}><a:off x="0" y="0"/><a:ext cx="{width * 12700}" cy="{height * 12700}"/></a:xfrm>
      <a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>'''


def png():
    # Four quadrants: red / green over blue / yellow. A reflection is
    # verifiable without a reference application's font or drawing choices.
    width, height = 80, 48
    colors = [(230, 40, 40), (30, 180, 80), (40, 90, 230), (245, 190, 25)]
    raw = b"".join(b"\0" + b"".join(bytes(colors[(y >= height // 2) * 2 + (x >= width // 2)]) for x in range(width)) for y in range(height))
    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


PAGE = '<w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="600" w:footer="600"/>'
LINKS = '<w:headerReference w:type="default" r:id="header"/><w:footerReference w:type="default" r:id="footer"/>'
STYLES = f'''<w:styles xmlns:w="{W}"><w:docDefaults><w:rPrDefault><w:rPr>
  <w:rFonts w:ascii="Liberation Serif" w:hAnsi="Liberation Serif" w:eastAsia="Source Han Serif CN"/><w:sz w:val="24"/>
  </w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="100"/></w:pPr></w:pPrDefault></w:docDefaults>
  <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:pPr><w:keepNext/><w:spacing w:before="160" w:after="160"/></w:pPr>
  <w:rPr><w:b/><w:sz w:val="36"/><w:color w:val="FF0000" w:themeColor="accent1" w:themeShade="BF"/></w:rPr></w:style></w:styles>'''
THEME = f'''<a:theme xmlns:a="{A}" name="Display probes"><a:themeElements><a:clrScheme name="Display probes">
  <a:dk1><a:sysClr val="windowText" lastClr="111111"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1>
  <a:accent1><a:srgbClr val="4F81BD"/></a:accent1><a:accent2><a:srgbClr val="C0504D"/></a:accent2>
  </a:clrScheme></a:themeElements></a:theme>'''
NUMBERING = f'''<w:numbering xmlns:w="{W}"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0">
  <w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:suff w:val="space"/>
  <w:rPr><w:color w:val="000000" w:themeColor="accent2"/></w:rPr></w:lvl></w:abstractNum>
  <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>'''


def package(name, body, section):
    def relations(entries):
        return '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' + "".join(
            f'<Relationship Id="{id}" Type="{R}/{kind}" Target="{target}"/>' for id, kind, target in entries
        ) + "</Relationships>"
    types = {"document": "document.main", "styles": "styles", "numbering": "numbering", "settings": "settings", "header": "header", "footer": "footer"}
    content_types = '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="png" ContentType="image/png"/>' + "".join(
        f'<Override PartName="/word/{name}.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.{kind}+xml"/>' for name, kind in types.items()
    ) + '<Override PartName="/word/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/></Types>'
    header = paragraph(picture('flipH="1"', width=18, height=12) + run("  显示细节 · ") + field(r"PAGE \* Arabic"), '<w:pBdr><w:bottom w:val="single" w:sz="4" w:themeColor="accent1"/></w:pBdr>')
    footer = paragraph(run("页 ") + field("PAGE") + run(" · 本节 ") + field("SECTIONPAGES") + run(" 页 · 全文 ") + field("NUMPAGES") + run(" 页 · 罗马页码 ") + field(r"PAGE \* ROMAN") + run(" · 英文总页数 ") + field(r"NUMPAGES \* CardText"), '<w:jc w:val="center"/>')
    parts = {
        "[Content_Types].xml": content_types,
        "_rels/.rels": relations([("main", "officeDocument", "word/document.xml")]),
        "word/document.xml": f'<w:document {NS}><w:body>{body}<w:sectPr>{section}</w:sectPr></w:body></w:document>',
        "word/_rels/document.xml.rels": relations([(key, key, f"{key}.xml") for key in ["styles", "numbering", "settings", "header", "footer"]] + [("theme", "theme", "theme/theme1.xml"), ("image", "image", "media/quadrants.png")]),
        "word/header.xml": f'<w:hdr {NS}>{header}</w:hdr>',
        "word/footer.xml": f'<w:ftr {NS}>{footer}</w:ftr>',
        "word/_rels/header.xml.rels": relations([("image", "image", "media/quadrants.png")]),
        "word/styles.xml": STYLES, "word/theme/theme1.xml": THEME, "word/numbering.xml": NUMBERING,
        "word/settings.xml": f'<w:settings xmlns:w="{W}"><w:compat><w:compatSetting w:name="compatibilityMode" w:val="15"/></w:compat></w:settings>',
        "word/media/quadrants.png": png(),
    }
    with zipfile.ZipFile(OUT / name, "w") as archive:
        for path, data in parts.items():
            entry = zipfile.ZipInfo(path, date_time=(2026, 1, 1, 0, 0, 0))
            entry.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(entry, data.encode() if isinstance(data, str) else data)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    body = paragraph(run("主题色标题与编号"), '<w:pStyle w:val="Heading1"/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>')
    body += paragraph(run("标题应为深蓝，编号为红褐；保存的回退色故意与主题色不同。"))
    for label, kind in [("仅文字下划线", "words"), ("粗虚线", "dashedHeavy"), ("双波浪线", "wavyDouble")]:
        body += paragraph(run(label + "：") + run("alpha beta  中文 空白", f'<w:u w:val="{kind}" w:themeColor="accent2"/>'))
    body += paragraph(run("直接颜色覆盖标题样式", '<w:color w:val="228833"/>'), '<w:pStyle w:val="Heading1"/>')
    body += paragraph(run("本节只有一页。"), f'<w:sectPr>{LINKS}{PAGE}</w:sectPr>')
    body += paragraph(run("图片镜像与裁切"), '<w:pStyle w:val="Heading1"/>')
    for label, flip, crop in [("原图", "", ""), ("水平镜像", 'flipH="1"', ""), ("垂直镜像", 'flipV="1"', ""), ("双向镜像", 'flipH="1" flipV="1"', ""), ("裁掉左半后水平镜像", 'flipH="1"', 'l="50000"')]:
        body += paragraph(run(label + "  ") + picture(flip, crop, width=90, height=45))
    body += paragraph(run("同节第二页"), '<w:pageBreakBefore/><w:pStyle w:val="Heading1"/>')
    body += paragraph(run("页脚：本节应为 2 页，全文应为 3 页；分节页码从 iv 开始，此页为 v。"))
    package("display-details.docx", body, PAGE + '<w:pgNumType w:start="4" w:fmt="lowerRoman"/>')
    print(OUT / "display-details.docx")


if __name__ == "__main__":
    main()
