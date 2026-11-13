#!/usr/bin/env python3
"""Small display audit inputs. Generated files are not reference answers."""
from pathlib import Path
import importlib.util
import json
import zipfile
from xml.sax.saxutils import escape

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "out/display-audit/probes"
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
A = "http://schemas.openxmlformats.org/drawingml/2006/main"
WP = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
PIC = "http://schemas.openxmlformats.org/drawingml/2006/picture"
NS = f'xmlns:w="{W}" xmlns:r="{R}" xmlns:a="{A}" xmlns:wp="{WP}" xmlns:pic="{PIC}"'
PAGE = '<w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="600" w:footer="600"/>'


def run(text, props=""):
    return f'<w:r><w:rPr>{props}</w:rPr><w:t xml:space="preserve">{escape(text)}</w:t></w:r>'


def para(content, props=""):
    return f"<w:p><w:pPr>{props}</w:pPr>{content}</w:p>"


def make(name, body, section=PAGE, media=None, extra=None):
    rels = [f'<Relationship Id="s" Type="{R}/styles" Target="styles.xml"/>']
    parts = {
        "word/document.xml": f'<w:document {NS}><w:body>{body}<w:sectPr>{section}</w:sectPr></w:body></w:document>',
        "word/styles.xml": f'<w:styles xmlns:w="{W}"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Liberation Serif" w:hAnsi="Liberation Serif" w:eastAsia="Source Han Serif CN"/><w:sz w:val="24"/></w:rPr></w:rPrDefault></w:docDefaults></w:styles>',
    }
    overrides = '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>'
    if media:
        rels.append(f'<Relationship Id="image" Type="{R}/image" Target="media/quadrants.png"/>')
        parts["word/media/quadrants.png"] = media
    for kind, data in (extra or {}).items():
        parts[f"word/{kind}.xml"] = data
        rels.append(f'<Relationship Id="{kind}" Type="{R}/{kind}" Target="{kind}.xml"/>')
        overrides += f'<Override PartName="/word/{kind}.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.{kind}+xml"/>'
    parts["word/_rels/document.xml.rels"] = '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' + "".join(rels) + '</Relationships>'
    parts["_rels/.rels"] = f'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="{R}/officeDocument" Target="word/document.xml"/></Relationships>'
    parts["[Content_Types].xml"] = '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="png" ContentType="image/png"/>' + overrides + '</Types>'
    with zipfile.ZipFile(OUT / (name + ".docx"), "w") as z:
        for path, data in parts.items():
            item = zipfile.ZipInfo(path, date_time=(2026, 1, 1, 0, 0, 0)); item.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(item, data.encode() if isinstance(data, str) else data)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    metadata = {}
    def record(name, question): metadata[name + ".docx"] = question
    for kind in ["oddPage", "evenPage"]:
        name = "section-" + kind
        body = para(run("BEFORE SECTION"), f'<w:sectPr>{PAGE}</w:sectPr>') + para(run("AFTER SECTION"))
        make(name, body, PAGE + f'<w:type w:val="{kind}"/>')
        record(name, "奇偶页分节是否插入所需空白页")

    spec = importlib.util.spec_from_file_location("display_details", ROOT / "tools/gen_display_details.py")
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    for rotation in [0, 5400000]:
        name = "picture-rotation-" + str(rotation)
        drawing = module.picture().replace('<a:xfrm ', f'<a:xfrm rot="{rotation}" ')
        make(name, para(run("PICTURE")) + para(drawing), media=module.png())
        record(name, "90 度旋转与未旋转的图像是否有区别")

    for style in ["plain", "outline", "shadow"]:
        name = "text-effect-" + style
        make(name, para(run("VISIBLE EFFECT", '<w:sz w:val="72"/>' + (f'<w:{style}/>' if style != 'plain' else ''))))
        record(name, "空心和阴影效果是否区别于普通文字")
    for style in ["single", "double"]:
        name = "paragraph-border-" + style
        make(name, para(run("BORDER"), f'<w:pBdr><w:bottom w:val="{style}" w:sz="16" w:space="4"/></w:pBdr>'))
        record(name, "双线边框是否真的绘制两条线")

    field = '<w:fldSimple w:instr="PAGE"><w:r><w:t>99</w:t></w:r></w:fldSimple>'
    make("body-page-fields", para(run("PAGE=") + field) + para(run("PAGE=") + field, '<w:pageBreakBefore/>'))
    record("body-page-fields", "正文页码是否仍显示保存值 99")

    box = f'''<w:r><w:drawing><wp:inline><wp:extent cx="2540000" cy="254000"/><wp:docPr id="1" name="box"/>
    <a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">
    <wps:cNvSpPr txBox="1"/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="2540000" cy="254000"/></a:xfrm>
    <a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/><a:ln w="12700"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:ln></wps:spPr>
    <wps:txbx><w:txbxContent>{''.join(para(run('BOX LINE ' + str(i))) for i in range(1, 5))}</w:txbxContent></wps:txbx>
    <wps:bodyPr lIns="0" tIns="0" rIns="0" bIns="0" vertOverflow="clip" horzOverflow="clip" anchor="t"><a:noAutofit/></wps:bodyPr></wps:wsp></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>'''
    make("textbox-overflow", para(box) + para(run("BODY AFTER THE BOX")))
    record("textbox-overflow", "超出文本框高度的文字是否遮盖后续正文")

    table = '<w:tbl><w:tblPr><w:tblLayout w:type="fixed"/></w:tblPr><w:tblGrid><w:gridCol w:w="5000"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="240" w:hRule="exact"/></w:trPr><w:tc><w:tcPr/>' + ''.join(para(run('CELL LINE ' + str(i))) for i in range(1, 5)) + '</w:tc></w:tr></w:tbl>'
    make("exact-row-clipping", table + para(run("BODY AFTER THE TABLE")))
    record("exact-row-clipping", "固定行高单元格内容是否被裁切")

    text = "Column content alpha beta gamma delta. " * 100
    make("two-columns", para(run(text)), PAGE + '<w:cols w:num="2" w:space="720"/>')
    record("two-columns", "分栏显示及未支持内容提示是否准确")

    make("table-vertical-text", '<w:tbl><w:tblGrid><w:gridCol w:w="1600"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:textDirection w:val="tbRl"/></w:tcPr>' + para(run("VERTICAL")) + '</w:tc></w:tr></w:tbl>')
    record("table-vertical-text", "单元格文字方向是否生效")
    (OUT / "questions.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2))
    print(f"{len(metadata)} probes -> {OUT}")


if __name__ == "__main__": main()
