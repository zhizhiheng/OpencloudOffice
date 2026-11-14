#!/usr/bin/env python3
"""Generate the reproducible DOCX used by the 0.0.1 release screenshots."""
from pathlib import Path
from xml.sax.saxutils import escape
import zipfile

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "testdata/release/opencloudoffice-0.0.1.docx"
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
A = "http://schemas.openxmlformats.org/drawingml/2006/main"
WP = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
PIC = "http://schemas.openxmlformats.org/drawingml/2006/picture"


def run(text, props=""):
    return f'<w:r><w:rPr>{props}</w:rPr><w:t xml:space="preserve">{escape(text)}</w:t></w:r>'


def para(content, style="Normal", extra=""):
    return f'<w:p><w:pPr><w:pStyle w:val="{style}"/>{extra}</w:pPr>{content}</w:p>'


def table(rows):
    widths = [2500, 1650, 5156]
    borders = "".join(f'<w:{s} w:val="single" w:sz="4" w:color="D7E1EF"/>' for s in ["top", "left", "bottom", "right", "insideH", "insideV"])
    result = f'<w:tbl><w:tblPr><w:tblW w:w="9306" w:type="dxa"/><w:tblLayout w:type="fixed"/><w:tblBorders>{borders}</w:tblBorders><w:tblCellMar><w:top w:w="90" w:type="dxa"/><w:bottom w:w="90" w:type="dxa"/><w:left w:w="140" w:type="dxa"/><w:right w:w="140" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid>'
    result += "".join(f'<w:gridCol w:w="{v}"/>' for v in widths) + "</w:tblGrid>"
    for index, row in enumerate(rows):
        result += '<w:tr><w:trPr><w:cantSplit/>' + ('<w:tblHeader/>' if index == 0 else '') + '</w:trPr>'
        for width, text in zip(widths, row):
            fill = "EAF1FB" if index == 0 else ("F6F8FC" if index % 2 == 0 else "FFFFFF")
            props = '<w:b/><w:color w:val="143B70"/>' if index == 0 else ''
            result += f'<w:tc><w:tcPr><w:tcW w:w="{width}" w:type="dxa"/><w:shd w:fill="{fill}"/></w:tcPr>{para(run(text, props), "Table")}</w:tc>'
        result += '</w:tr>'
    return result + '</w:tbl>'


def field(name):
    return f'<w:fldSimple w:instr="{name}">{run("1")}</w:fldSimple>'


def reference(number):
    return f'<w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:footnoteReference w:id="{number}"/></w:r>'


def main():
    styles = f'''<w:styles xmlns:w="{W}">
    <w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Liberation Sans" w:hAnsi="Liberation Sans" w:eastAsia="Source Han Serif CN"/><w:sz w:val="22"/><w:color w:val="273549"/><w:lang w:val="en-US" w:eastAsia="zh-CN"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="120" w:line="320" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults>
    <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
    <w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:before="40" w:after="120"/><w:keepNext/></w:pPr><w:rPr><w:rFonts w:eastAsia="Source Han Sans CN"/><w:b/><w:sz w:val="52"/><w:color w:val="12396A"/></w:rPr></w:style>
    <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="Heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:before="200" w:after="120"/><w:keepNext/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:rFonts w:eastAsia="Source Han Sans CN"/><w:b/><w:sz w:val="30"/><w:color w:val="175CB2"/></w:rPr></w:style>
    <w:style w:type="paragraph" w:styleId="Table"><w:name w:val="Table"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:after="0" w:line="280" w:lineRule="auto"/></w:pPr><w:rPr><w:sz w:val="20"/></w:rPr></w:style>
    <w:style w:type="paragraph" w:styleId="Small"><w:name w:val="Small"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:after="80" w:line="260" w:lineRule="auto"/></w:pPr><w:rPr><w:sz w:val="18"/><w:color w:val="64748B"/></w:rPr></w:style>
    <w:style w:type="paragraph" w:styleId="FootnoteText"><w:name w:val="footnote text"/><w:basedOn w:val="Small"/><w:pPr><w:spacing w:after="80"/></w:pPr></w:style></w:styles>'''
    cx, cy = 5905500, 3323670
    image = f'''<w:r><w:drawing><wp:inline><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="1" name="OpencloudOffice product impression" descr="ImageGen 产品印象图"/>
    <a:graphic><a:graphicData uri="{PIC}"><pic:pic><pic:nvPicPr><pic:cNvPr id="1" name="hero.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="hero"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>'''
    body = para(run("FIRST OPEN-SOURCE RELEASE  /  2026"), "Small")
    body += para(run("让文档，在浏览器里展开。"), "Title")
    body += para(run("OpencloudOffice 0.0.1  ·  自研文档显示内核  ·  Apache-2.0"), "Small")
    body += para(image, extra='<w:spacing w:after="80"/>')
    body += para(run("01  从一份文档开始"), "Heading1")
    body += para(run("读取 DOCX，在本机完成排版与绘制。页眉页脚、标题、表格、图片与脚注，共同组成一张可以阅读、选择和查找的页面。"))
    body += table([
        ["组成", "运行位置", "本次交付"],
        ["文档显示内核", "Rust / WASM", "格式读取、分页、字形与 SVG 绘制"],
        ["浏览器查看器", "浏览器本地", "全文查找、跳页、缩放和文字复制"],
        ["开发工具", "命令行", "文档检查、页面导出和对照测试"],
    ])
    body += para(run("这是一份真实 DOCX 示例；上方封面插图由 ImageGen 生成。"), "Small")
    body += para(run("02  阅读中的每个细节"), "Title", '<w:pageBreakBefore/>')
    body += para(run("显示先行。我们先把文字与页面的关系做清楚，再逐步扩展编辑和写回。下面的脚注由同一套自研排版内核放置在页面底部。") + reference(1))
    body += para(run("一套内核，两个运行环境"), "Heading1")
    for text in ["Rust 原生与 WebAssembly 共用读取、布局和绘制代码。", "浏览器后台线程负责排版，查看器按可见页绘制。", "文件在本地处理；可为文档补充本机字体。"]:
        body += para(run(text), extra='<w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>')
    body += para(run("文字、层级与强调"), "Heading1")
    body += para(run("清楚的标题，") + run("明确的重点", '<w:b/><w:color w:val="175CB2"/>') + run("，以及 ") + run("readable typography", '<w:i/>') + run("。"))
    body += para(run("下划线标记", '<w:u w:val="single"/>') + run("  ·  ") + run("主题色强调", '<w:color w:val="12846F"/>') + run("  ·  H") + run("2", '<w:vertAlign w:val="subscript"/>') + run("O  ·  x") + run("2", '<w:vertAlign w:val="superscript"/>'))
    body += para(run("对照结果与实现范围"), "Heading1")
    body += table([
        ["检查", "样本", "0.0.1 结果"],
        ["Word 行位置", "128 份", "83.6% 在 0.5 pt 误差内"],
        ["WPS 中文页数", "11 份", "11 / 11 页数一致"],
        ["原生与 WASM", "163 份", "403 页 SVG 输出一致"],
    ])
    body += para(run("行级对照用于定位差异，图像、边框和裁切仍需单独检查。") + reference(2))
    body += para(run("开发者预览：分栏、复杂浮动对象、图片旋转和内容裁切继续迭代，编辑与写回安排在后续阶段。"), "Small")
    header = f'<w:hdr xmlns:w="{W}">' + para(run("OpencloudOffice   /   DOCUMENT PREVIEW   /   0.0.1"), "Small", '<w:pBdr><w:bottom w:val="single" w:sz="4" w:space="6" w:color="D7E1EF"/></w:pBdr>') + '</w:hdr>'
    footer = f'<w:ftr xmlns:w="{W}">' + para(run("OpencloudOffice  ·  ") + field("PAGE") + run(" / ") + field("NUMPAGES"), "Small", '<w:jc w:val="center"/>') + '</w:ftr>'
    notes = f'<w:footnotes xmlns:w="{W}"><w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote>'
    for number, text in [(1, "本示例展示页底脚注、引用编号与正文占位。长脚注还可以跨页延续。"), (2, "数字来自固定样本，不代表所有文档的视觉还原率。完整方法和限制见仓库的显示能力审计。")]:
        mark = '<w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:footnoteRef/></w:r>'
        notes += f'<w:footnote w:id="{number}">' + para(mark + run(" " + text), "FootnoteText") + '</w:footnote>'
    notes += '</w:footnotes>'
    numbering = f'<w:numbering xmlns:w="{W}"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:lvlJc w:val="left"/><w:pPr><w:tabs><w:tab w:val="num" w:pos="360"/></w:tabs><w:ind w:left="360" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>'
    section = '<w:sectPr><w:headerReference w:type="default" r:id="header"/><w:footerReference w:type="default" r:id="footer"/><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1200" w:right="1300" w:bottom="1200" w:left="1300" w:header="560" w:footer="560"/></w:sectPr>'
    settings = f'<w:settings xmlns:w="{W}"><w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat></w:settings>'
    document = f'<w:document xmlns:w="{W}" xmlns:r="{R}" xmlns:a="{A}" xmlns:wp="{WP}" xmlns:pic="{PIC}"><w:body>{body}{section}</w:body></w:document>'
    parts = {"document": document, "styles": styles, "settings": settings, "header": header, "footer": footer, "footnotes": notes, "numbering": numbering}
    ct = "http://schemas.openxmlformats.org/package/2006/content-types"
    rel = "http://schemas.openxmlformats.org/package/2006/relationships"
    content_types = f'<Types xmlns="{ct}"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="png" ContentType="image/png"/>'
    for name in parts:
        kind = 'document.main' if name == 'document' else name
        content_types += f'<Override PartName="/word/{name}.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.{kind}+xml"/>'
    content_types += '</Types>'
    relationships = f'<Relationships xmlns="{rel}">'
    for name in parts:
        if name != 'document':
            relationships += f'<Relationship Id="{name}" Type="{R}/{name}" Target="{name}.xml"/>'
    relationships += f'<Relationship Id="hero" Type="{R}/image" Target="media/hero.png"/></Relationships>'
    files = {f'word/{name}.xml': text.encode() for name, text in parts.items()}
    files.update({
        '[Content_Types].xml': content_types.encode(),
        '_rels/.rels': f'<Relationships xmlns="{rel}"><Relationship Id="main" Type="{R}/officeDocument" Target="word/document.xml"/></Relationships>'.encode(),
        'word/_rels/document.xml.rels': relationships.encode(),
        'word/media/hero.png': (ROOT / 'docs/images/opencloudoffice-0.0.1-hero.png').read_bytes(),
    })
    OUT.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(OUT, 'w') as archive:
        for path, data in files.items():
            entry = zipfile.ZipInfo(path, date_time=(2026, 11, 14, 0, 0, 0))
            entry.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(entry, data)
    print(OUT)


if __name__ == '__main__':
    main()
