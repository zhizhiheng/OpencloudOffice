"""A minimal .docx writer for building layout test documents by hand.

Every part of the package is written out explicitly, so a test document states
exactly the markup it depends on: styles, numbering definitions, page setup
and document grid. Defaults follow what the Chinese edition of Word writes for
a new document (A4, 五号 body text, a line grid of 15.6 pt).

Lengths are in twips (1/20 pt) and font sizes in points unless noted.
"""

import zipfile
from pathlib import Path
from xml.sax.saxutils import escape

W_NS = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
W = f'xmlns:w="{W_NS}"'

# Chinese type sizes (字号) in points.
SIZES = {
    "初号": 42, "小初": 36, "一号": 26, "小一": 24, "二号": 22, "小二": 18, "三号": 16, "小三": 15,
    "四号": 14, "小四": 12, "五号": 10.5, "小五": 9, "六号": 7.5, "小六": 6.5, "七号": 5.5, "八号": 5,
}


def _on(name, value):
    if value is None:
        return ""
    return f"<w:{name}/>" if value else f'<w:{name} w:val="0"/>'


def rpr(*, style=None, font=None, ascii=None, sz=None, b=None, i=None, u=None, u_color=None, strike=None,
        dstrike=None, color=None, highlight=None, vert=None, em=None, spacing=None, scale=None, position=None,
        shd=None, bdr=None, caps=None, small_caps=None, vanish=None, kern=None, hint=False):
    """Run properties. `font` sets the East Asian font, `ascii` the Latin one.

    `hint` marks the run as East Asian text, which Word does for text typed
    in Chinese; it makes curly quotes and dashes take the Chinese font.
    """
    out = []
    if style:
        out.append(f'<w:rStyle w:val="{style}"/>')
    if font or ascii or hint:
        attrs = ""
        if ascii:
            attrs += f' w:ascii="{ascii}" w:hAnsi="{ascii}"'
        if font:
            attrs += f' w:eastAsia="{font}"'
        if hint:
            attrs += ' w:hint="eastAsia"'
        out.append(f"<w:rFonts{attrs}/>")
    out += [_on("b", b), _on("bCs", b), _on("i", i), _on("iCs", i), _on("caps", caps), _on("smallCaps", small_caps)]
    out += [_on("strike", strike), _on("dstrike", dstrike), _on("vanish", vanish)]
    if color:
        out.append(f'<w:color w:val="{color}"/>')
    if spacing is not None:
        out.append(f'<w:spacing w:val="{spacing}"/>')
    if scale is not None:
        out.append(f'<w:w w:val="{scale}"/>')
    if kern is not None:
        out.append(f'<w:kern w:val="{kern}"/>')
    if position is not None:
        out.append(f'<w:position w:val="{position}"/>')
    if sz is not None:
        half = round(sz * 2)
        out.append(f'<w:sz w:val="{half}"/><w:szCs w:val="{half}"/>')
    if highlight:
        out.append(f'<w:highlight w:val="{highlight}"/>')
    if u:
        colour = f' w:color="{u_color}"' if u_color else ""
        out.append(f'<w:u w:val="{u}"{colour}/>')
    if bdr:
        out.append('<w:bdr w:val="single" w:sz="4" w:space="0" w:color="auto"/>')
    if shd:
        out.append(f'<w:shd w:val="clear" w:color="auto" w:fill="{shd}"/>')
    if vert:
        out.append(f'<w:vertAlign w:val="{vert}"/>')
    if em:
        out.append(f'<w:em w:val="{em}"/>')
    inner = "".join(out)
    return f"<w:rPr>{inner}</w:rPr>" if inner else ""


def r(text="", **fmt):
    """A run. `\\t` becomes a tab and `\\n` a line break."""
    parts = []
    buffer = ""
    for ch in text:
        if ch in "\t\n":
            if buffer:
                parts.append(f'<w:t xml:space="preserve">{escape(buffer)}</w:t>')
                buffer = ""
            parts.append("<w:tab/>" if ch == "\t" else "<w:br/>")
        else:
            buffer += ch
    if buffer:
        parts.append(f'<w:t xml:space="preserve">{escape(buffer)}</w:t>')
    fmt.setdefault("hint", any("\u2e80" <= ch <= "\u9fff" or "\uff00" <= ch <= "\uffef" for ch in text))
    return f"<w:r>{rpr(**fmt)}{''.join(parts)}</w:r>"


def page_break():
    return '<w:r><w:br w:type="page"/></w:r>'


def ppr(*, style=None, jc=None, ind=None, spacing=None, num=None, keep_next=None, keep_lines=None,
        page_break_before=None, tabs=None, mark=None, border_bottom=None, snap=None, outline=None, widow=None):
    out = []
    if style:
        out.append(f'<w:pStyle w:val="{style}"/>')
    out += [_on("keepNext", keep_next), _on("keepLines", keep_lines), _on("pageBreakBefore", page_break_before)]
    out.append(_on("widowControl", widow))
    if num:
        out.append(f'<w:numPr><w:ilvl w:val="{num[1]}"/><w:numId w:val="{num[0]}"/></w:numPr>')
    if border_bottom:
        size, colour, space = border_bottom
        out.append(f'<w:pBdr><w:bottom w:val="single" w:sz="{size}" w:space="{space}" w:color="{colour}"/></w:pBdr>')
    if tabs:
        stops = "".join(
            f'<w:tab w:val="{kind}" w:pos="{pos}"' + (f' w:leader="{leader}"' if leader else "") + "/>"
            for kind, pos, leader in tabs
        )
        out.append(f"<w:tabs>{stops}</w:tabs>")
    out.append(_on("snapToGrid", snap))
    if spacing:
        attrs = "".join(f' w:{k}="{v}"' for k, v in spacing.items())
        out.append(f"<w:spacing{attrs}/>")
    if ind:
        attrs = "".join(f' w:{k}="{v}"' for k, v in ind.items())
        out.append(f"<w:ind{attrs}/>")
    if jc:
        out.append(f'<w:jc w:val="{jc}"/>')
    if outline is not None:
        out.append(f'<w:outlineLvl w:val="{outline}"/>')
    if mark:
        out.append(mark)
    inner = "".join(out)
    return f"<w:pPr>{inner}</w:pPr>" if inner else ""


def p(*runs, **props):
    """A paragraph. Plain strings among `runs` become unformatted runs."""
    body = "".join(run if run.startswith("<") else r(run) for run in runs)
    return f"<w:p>{ppr(**props)}{body}</w:p>"


def style(style_id, name, *, kind="paragraph", based_on=None, next_style=None, default=False, ppr_xml="", rpr_xml=""):
    attrs = f' w:type="{kind}" w:styleId="{style_id}"' + (' w:default="1"' if default else "")
    inner = f'<w:name w:val="{name}"/>'
    if based_on:
        inner += f'<w:basedOn w:val="{based_on}"/>'
    if next_style:
        inner += f'<w:next w:val="{next_style}"/>'
    return f"<w:style{attrs}>{inner}{ppr_xml}{rpr_xml}</w:style>"


def default_styles(*, body_font="宋体", latin_font="Times New Roman", body_size=10.5, extra="", heading_num=None):
    """The style sheet of a new document in Chinese Word, with fonts named directly.

    With `heading_num`, the three heading styles take their numbers from that
    numbering instance (levels 0 to 2), which is how numbered headings work.
    """
    half = round(body_size * 2)
    numbered = lambda ilvl: (heading_num, ilvl) if heading_num is not None else None
    return (
        f"<w:styles {W}><w:docDefaults><w:rPrDefault><w:rPr>"
        f'<w:rFonts w:ascii="{latin_font}" w:hAnsi="{latin_font}" w:eastAsia="{body_font}" w:cs="{latin_font}"/>'
        f'<w:kern w:val="2"/><w:sz w:val="{half}"/><w:szCs w:val="22"/>'
        '<w:lang w:val="en-US" w:eastAsia="zh-CN" w:bidi="ar-SA"/>'
        "</w:rPr></w:rPrDefault><w:pPrDefault/></w:docDefaults>"
        + style("a", "Normal", default=True, ppr_xml=ppr(widow=False, jc="both"))
        + style("a0", "Default Paragraph Font", kind="character", default=True)
        + style(
            "1", "heading 1", based_on="a", next_style="a",
            ppr_xml=ppr(keep_next=True, keep_lines=True, num=numbered(0), spacing={"before": 340, "after": 330, "line": 578, "lineRule": "auto"}, outline=0),
            rpr_xml=rpr(b=True, kern=44, sz=22),
        )
        + style(
            "2", "heading 2", based_on="a", next_style="a",
            ppr_xml=ppr(keep_next=True, keep_lines=True, num=numbered(1), spacing={"before": 260, "after": 260, "line": 416, "lineRule": "auto"}, outline=1),
            rpr_xml=rpr(font="黑体", ascii="Arial", b=True, sz=16),
        )
        + style(
            "3", "heading 3", based_on="a", next_style="a",
            ppr_xml=ppr(keep_next=True, keep_lines=True, num=numbered(2), spacing={"before": 260, "after": 260, "line": 416, "lineRule": "auto"}, outline=2),
            rpr_xml=rpr(b=True, sz=16),
        )
        + extra
        + "</w:styles>"
    )


def level(ilvl, fmt, text, *, start=1, jc="left", left=0, hanging=0, first_line=None, suff=None, rpr_xml="",
          pstyle=None, restart=None, legal=False, tab=None):
    out = [f'<w:lvl w:ilvl="{ilvl}"><w:start w:val="{start}"/><w:numFmt w:val="{fmt}"/>']
    if restart is not None:
        out.append(f'<w:lvlRestart w:val="{restart}"/>')
    if pstyle:
        out.append(f'<w:pStyle w:val="{pstyle}"/>')
    if legal:
        out.append("<w:isLgl/>")
    if suff:
        out.append(f'<w:suff w:val="{suff}"/>')
    out.append(f'<w:lvlText w:val="{escape(text)}"/><w:lvlJc w:val="{jc}"/>')
    indent = f' w:left="{left}"'
    indent += f' w:firstLine="{first_line}"' if first_line is not None else f' w:hanging="{hanging}"'
    tabs = f'<w:tabs><w:tab w:val="num" w:pos="{tab}"/></w:tabs>' if tab is not None else ""
    out.append(f"<w:pPr>{tabs}<w:ind{indent}/></w:pPr>{rpr_xml}</w:lvl>")
    return "".join(out)


def abstract_num(abstract_id, levels, multi="multilevel"):
    return (
        f'<w:abstractNum w:abstractNumId="{abstract_id}"><w:multiLevelType w:val="{multi}"/>'
        + "".join(levels)
        + "</w:abstractNum>"
    )


def num(num_id, abstract_id, overrides=""):
    return f'<w:num w:numId="{num_id}"><w:abstractNumId w:val="{abstract_id}"/>{overrides}</w:num>'


def start_override(ilvl, start):
    return f'<w:lvlOverride w:ilvl="{ilvl}"><w:startOverride w:val="{start}"/></w:lvlOverride>'


def numbering(abstracts, nums):
    return f"<w:numbering {W}>{''.join(abstracts)}{''.join(nums)}</w:numbering>"


def section(*, width=11906, height=16838, top=1440, bottom=1440, left=1800, right=1800, grid=("lines", 312)):
    grid_xml = ""
    if grid:
        kind, pitch, *rest = grid
        char_space = f' w:charSpace="{rest[0]}"' if rest else ""
        grid_xml = f'<w:docGrid w:type="{kind}" w:linePitch="{pitch}"{char_space}/>'
    return (
        f'<w:sectPr><w:pgSz w:w="{width}" w:h="{height}"/>'
        f'<w:pgMar w:top="{top}" w:right="{right}" w:bottom="{bottom}" w:left="{left}" w:header="851" w:footer="992" w:gutter="0"/>'
        f'<w:cols w:space="425"/>{grid_xml}</w:sectPr>'
    )


SETTINGS = (
    f"<w:settings {W}><w:bordersDoNotSurroundHeader/><w:bordersDoNotSurroundFooter/>"
    '<w:defaultTabStop w:val="420"/><w:drawingGridVerticalSpacing w:val="156"/>'
    '<w:characterSpacingControl w:val="compressPunctuation"/>'
    "<w:compat><w:spaceForUL/><w:balanceSingleByteDoubleByteWidth/><w:doNotLeaveBackslashAlone/>"
    "<w:ulTrailSpace/><w:doNotExpandShiftReturn/><w:adjustLineHeightInTable/><w:useFELayout/>"
    '<w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/>'
    "</w:compat></w:settings>"
)

CONTENT_TYPES = (
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
    '<Default Extension="xml" ContentType="application/xml"/>'
    '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
    '<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>'
    '<Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>'
    '<Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>'
    '<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>'
    "</Types>"
)
RELS_NS = "http://schemas.openxmlformats.org/package/2006/relationships"
DOC_RELS = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
PACKAGE_RELS = (
    f'<Relationships xmlns="{RELS_NS}">'
    f'<Relationship Id="rId1" Type="{DOC_RELS}/officeDocument" Target="word/document.xml"/>'
    f'<Relationship Id="rId2" Type="{DOC_RELS}/extended-properties" Target="docProps/app.xml"/>'
    "</Relationships>"
)
DOCUMENT_RELS = (
    f'<Relationships xmlns="{RELS_NS}">'
    f'<Relationship Id="rId1" Type="{DOC_RELS}/styles" Target="styles.xml"/>'
    f'<Relationship Id="rId2" Type="{DOC_RELS}/settings" Target="settings.xml"/>'
    f'<Relationship Id="rId3" Type="{DOC_RELS}/numbering" Target="numbering.xml"/>'
    "</Relationships>"
)
APP = (
    '<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties">'
    "<Application>OpencloudOffice test corpus</Application></Properties>"
)


def write(path, body, *, styles=None, numbering_xml=None, sect=None):
    """Writes a package. Entries have a fixed timestamp so rebuilds are byte-identical."""
    parts = {
        "[Content_Types].xml": CONTENT_TYPES,
        "_rels/.rels": PACKAGE_RELS,
        "docProps/app.xml": APP,
        "word/_rels/document.xml.rels": DOCUMENT_RELS,
        "word/document.xml": f"<w:document {W}><w:body>{body}{sect or section()}</w:body></w:document>",
        "word/styles.xml": styles or default_styles(),
        "word/settings.xml": SETTINGS,
        "word/numbering.xml": numbering_xml or numbering([], []),
    }
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, content in parts.items():
            info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n' + content)
