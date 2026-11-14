#!/usr/bin/env python3
"""Writes the probe documents of testdata/probes: small documents that each
try a few rules of layout, made to be laid out by a reference application
and measured (tools/wps/export.py testdata/probes, then tools/fidelity/score.py).

    python3 tools/probes/make.py [testdata/probes]

The documents are written the same byte for byte each time. They are set in
Liberation Serif, and the Chinese ones in Source Han Serif CN (./fonts), which
the reference machine has, so that no font is replaced. What each document tries is said where it is made, and in
testdata/probes/README.md.
"""

import sys
import zipfile
from pathlib import Path

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
PACKAGE = "http://schemas.openxmlformats.org/package/2006"
FONT = "Liberation Serif"
PAGE = (
    '<w:pgSz w:w="11906" w:h="16838"/>'
    '<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708"/>'
)
LOREM = (
    "The quick brown fox jumps over the lazy dog while the farmer watches from the gate. "
    "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor."
)


def run_props(size=24, extra=""):
    fonts = f'<w:rFonts w:ascii="{FONT}" w:hAnsi="{FONT}" w:eastAsia="{FONT}" w:cs="{FONT}"/>'
    return f'<w:rPr>{fonts}{extra}<w:sz w:val="{size}"/><w:szCs w:val="{size}"/></w:rPr>'


def spacing(before=0, after=0):
    return f'<w:spacing w:before="{before}" w:after="{after}" w:line="240" w:lineRule="auto"/>'


def write(path, body, last_section="", footnotes=None, settings="", styles=None, page=PAGE):
    """A package with the given body; with a footnotes part and settings, or
    with styles, where given. `page` is the paper and margins of the last section."""
    parts = {
        "word/document.xml": f'<w:document xmlns:w="{W}" xmlns:r="{R}"><w:body>{body}'
        f"<w:sectPr>{last_section}{page}</w:sectPr></w:body></w:document>"
    }
    kinds = {"word/document.xml": "document.main"}
    relationships = []
    if styles is not None:
        parts["word/styles.xml"] = f'<w:styles xmlns:w="{W}">{styles}</w:styles>'
        kinds["word/styles.xml"] = "styles"
        relationships.append(("rId3", "styles", "styles.xml"))
    if footnotes is not None:
        parts["word/footnotes.xml"] = f'<w:footnotes xmlns:w="{W}">{footnotes}</w:footnotes>'
        mode = '<w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/>'
        parts["word/settings.xml"] = f'<w:settings xmlns:w="{W}">{settings}<w:compat>{mode}</w:compat></w:settings>'
        kinds.update({"word/footnotes.xml": "footnotes", "word/settings.xml": "settings"})
        relationships += [("rId1", "footnotes", "footnotes.xml"), ("rId2", "settings", "settings.xml")]
    wordprocessing = "application/vnd.openxmlformats-officedocument.wordprocessingml"
    overrides = "".join(
        f'<Override PartName="/{name}" ContentType="{wordprocessing}.{kind}+xml"/>' for name, kind in kinds.items()
    )
    files = {
        "[Content_Types].xml": f'<Types xmlns="{PACKAGE}/content-types">'
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        f'<Default Extension="xml" ContentType="application/xml"/>{overrides}</Types>',
        "_rels/.rels": f'<Relationships xmlns="{PACKAGE}/relationships">'
        f'<Relationship Id="rId1" Type="{R}/officeDocument" Target="word/document.xml"/></Relationships>',
    }
    if relationships:
        files["word/_rels/document.xml.rels"] = f'<Relationships xmlns="{PACKAGE}/relationships">' + "".join(
            f'<Relationship Id="{id}" Type="{R}/{kind}" Target="{target}"/>' for id, kind, target in relationships
        ) + "</Relationships>"
    files.update(parts)
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as package:
        for name, content in files.items():
            # A fixed date: the same bytes each time.
            entry = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            entry.compress_type = zipfile.ZIP_DEFLATED
            package.writestr(entry, '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' + content)


def edge(name, size=8, space=1, color="000000", val="single"):
    return f'<w:{name} w:val="{val}" w:sz="{size}" w:space="{space}" w:color="{color}"/>'


def paragraph(text, borders="", shade="", indent="", before=0, after=0):
    borders = f"<w:pBdr>{borders}</w:pBdr>" if borders else ""
    shade = f'<w:shd w:val="clear" w:color="auto" w:fill="{shade}"/>' if shade else ""
    return (
        f"<w:p><w:pPr>{borders}{shade}{spacing(before, after)}{indent}</w:pPr>"
        f'<w:r>{run_props()}<w:t xml:space="preserve">{text}</w:t></w:r></w:p>'
    )


def paragraph_borders(path):
    """Borders and shading of paragraphs: where the lines and the fill are, the
    room they take, paragraphs boxed as one, borders with indents."""

    def box(size=8, space=1, between=False):
        sides = "".join(edge(name, size, space) for name in ("top", "left", "bottom", "right"))
        return sides + (edge("between", size, space) if between else "")

    def para(label, borders="", shade="", indent="", before=0, after=0, text=LOREM):
        return paragraph(f"{label} {text}", borders, shade, indent, before, after)

    short = "one line only."
    hanging = '<w:ind w:left="720" w:hanging="360"/>'
    body = "".join(
        [
            para("P01 plain.", after=120, text=short),
            para("P02 shade,", shade="D9D9D9", before=120, after=240),
            para("P03 plain.", after=120, text=short),
            para("P04 shade, indents,", shade="BDD7EE", indent='<w:ind w:left="720" w:right="720" w:firstLine="360"/>', after=120),
            para("P05 plain.", after=120, text=short),
            para("P06 box 1pt space 1,", borders=box(8, 1), after=240),
            para("P07 plain.", after=120, text=short),
            para("P08 box 1.5pt space 4 and shade, left 720,", borders=box(12, 4), shade="FFE699", indent='<w:ind w:left="720"/>', before=120, after=240),
            para("P09 plain.", after=120, text=short),
            para("P10 joined box,", borders=box(4, 4), shade="E2EFDA", after=120, text=short),
            para("P11 joined box,", borders=box(4, 4), shade="E2EFDA", after=120, text=short),
            para("P12 joined box,", borders=box(4, 4), shade="E2EFDA", after=120, text=short),
            para("P13 plain.", after=120, text=short),
            para("P14 box with between,", borders=box(4, 4, True), after=120, text=short),
            para("P15 box with between,", borders=box(4, 4, True), after=120, text=short),
            para("P16 plain.", after=120, text=short),
            para("P17 left 3pt space 4 and shade, hanging,", borders=edge("left", 24, 4), shade="F2F2F2", indent=hanging, after=120),
            para("P18 left 3pt space 4 and shade, hanging,", borders=edge("left", 24, 4), shade="F2F2F2", indent=hanging, after=120, text=short),
            para("P19 plain.", after=120, text=short),
            para("P20 top space 10 bottom space 1,", borders=edge("top", 8, 10) + edge("bottom", 8, 1), after=120, text=short),
            para("P21 plain.", after=120, text=short),
            para("P22 right border and shade, right 1440,", borders=edge("right", 8, 4), shade="DDEBF7", indent='<w:ind w:right="1440"/>', after=120),
            para("P23 after.", text=short),
        ]
    )
    write(path, body)


def shading_and_tables(path, cell_margins):
    """The fill of paragraphs through the space between them, and three tables:
    lines of cells against lines of the table, dotted and absent inside lines,
    no lines at all, fills of cells. With `cell_margins` the tables state the
    margins of their cells."""
    indent = '<w:ind w:left="720"/>'
    grey = "D9D9D9"
    body = "".join(
        [
            paragraph("A01 plain after 0"),
            paragraph("A02 shade before 0 after 240", shade=grey, after=240),
            paragraph("A03 plain"),
            paragraph("A04 shade before 120 after 120", shade=grey, before=120, after=120),
            paragraph("A05 plain"),
            paragraph("A06 shade before 240 after 0", shade=grey, before=240),
            paragraph("A07 plain"),
            paragraph("A08 shade after 120 left 720", shade=grey, indent=indent, after=120),
            paragraph("A09 plain before 240", before=240),
            paragraph("A10 shade after 120", shade=grey, after=120),
            paragraph("A11 plain before 240", before=240),
            paragraph("A12 shade before 120 after 240 left 720", shade=grey, indent=indent, before=120, after=240),
            paragraph("A13 plain"),
            paragraph("A14 shade after 60", shade=grey, after=60),
            paragraph("A15 plain"),
            paragraph("A16 shade after 200", shade=grey, after=200),
            paragraph("A17 plain"),
            paragraph("A18 shade after 480", shade=grey, after=480),
            paragraph("A19 plain", after=240),
        ]
    )

    def line(name, val="single", size=4, color="000000"):
        return edge(name, size, 0, color, val)

    def cell(text, shade="", borders="", paragraph_shade=""):
        shade = f'<w:shd w:val="clear" w:color="auto" w:fill="{shade}"/>' if shade else ""
        borders = f"<w:tcBorders>{borders}</w:tcBorders>" if borders else ""
        return f'<w:tc><w:tcPr><w:tcW w:w="2400" w:type="dxa"/>{borders}{shade}</w:tcPr>{paragraph(text, shade=paragraph_shade)}</w:tc>'

    all_lines = "".join(line(name) for name in ("top", "left", "bottom", "right", "insideH", "insideV"))
    margins = '<w:tblCellMar><w:left w:w="108" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar>'

    def table(rows, borders=all_lines):
        properties = (
            f'<w:tblW w:w="7200" w:type="dxa"/><w:tblBorders>{borders}</w:tblBorders>'
            f'<w:tblLayout w:type="fixed"/>{margins if cell_margins else ""}'
        )
        grid = '<w:gridCol w:w="2400"/>' * 3
        rows = "".join(f'<w:tr>{"".join(row)}</w:tr>' for row in rows)
        return f"<w:tbl><w:tblPr>{properties}</w:tblPr><w:tblGrid>{grid}</w:tblGrid>{rows}</w:tbl>"

    blue_box = "".join(line(name, size=12, color="0000FF") for name in ("top", "left", "bottom", "right"))
    first = table(
        [
            [cell("T1 head a", shade="BFBFBF"), cell("T1 head b", shade="BFBFBF"), cell("T1 head c", shade="BFBFBF")],
            [cell("T1 plain"), cell("T1 yellow", shade="FFFF00"), cell("T1 para shade", paragraph_shade="C6E0B4")],
            [
                cell("T1 thick red bottom", borders=line("bottom", size=24, color="FF0000")),
                cell("T1 no top", borders=line("top", val="nil")),
                cell("T1 blue box", borders=blue_box),
            ],
        ]
    )
    outer = "".join(line(name, size=12) for name in ("top", "left", "bottom", "right"))
    second = table(
        [
            [cell("T2 a", shade="DDEBF7"), cell("T2 b"), cell("T2 c")],
            [cell("T2 d"), cell("T2 e", shade="FCE4D6"), cell("T2 f")],
        ],
        borders=outer + line("insideH", val="dotted", color="808080") + line("insideV", val="nil"),
    )
    third = table(
        [
            [cell("T3 a", shade="E2EFDA"), cell("T3 b", shade="E2EFDA"), cell("T3 c")],
            [cell("T3 d"), cell("T3 e"), cell("T3 f", shade="E2EFDA")],
        ],
        borders="",
    )
    between = lambda text: paragraph(text, before=120, after=120)  # noqa: E731
    write(path, body + first + between("B01 between tables") + second + between("B02 between tables") + third + paragraph("B03 after"))


def footnotes(directory):
    """Footnotes at the foot of the page, and how they are numbered: 120 lines
    of text over three or four pages, eight of them with a note."""
    noted = [3, 10, 20, 55, 60, 70, 105, 110]
    raised = '<w:vertAlign w:val="superscript"/>'

    def body(first_section_ends_at=None, first_section=""):
        lines = []
        for number in range(1, 121):
            reference = ""
            if number in noted:
                id = noted.index(number) + 1
                reference = f'<w:r>{run_props(24, raised)}<w:footnoteReference w:id="{id}"/></w:r>'
            section = f"<w:sectPr>{first_section}{PAGE}</w:sectPr>" if number == first_section_ends_at else ""
            text = f'<w:r>{run_props()}<w:t xml:space="preserve">B{number:03d} body line</w:t></w:r>'
            lines.append(f"<w:p><w:pPr>{spacing()}{section}</w:pPr>{text}{reference}</w:p>")
        return "".join(lines)

    def rule(id, kind):
        return f'<w:footnote w:type="{kind}" w:id="{id}"><w:p><w:pPr>{spacing()}</w:pPr><w:r><w:{kind}/></w:r></w:p></w:footnote>'

    notes = rule(-1, "separator") + rule(0, "continuationSeparator")
    for index, number in enumerate(noted):
        mark = f"<w:r>{run_props(20, raised)}<w:footnoteRef/></w:r>"
        text = f'<w:r>{run_props(20)}<w:t xml:space="preserve"> note of line {number}</w:t></w:r>'
        notes += f'<w:footnote w:id="{index + 1}"><w:p><w:pPr>{spacing()}</w:pPr>{mark}{text}</w:p></w:footnote>'

    def of_document(inner):
        return f'<w:footnotePr>{inner}<w:footnote w:id="-1"/><w:footnote w:id="0"/></w:footnotePr>'

    def of_section(inner):
        return f"<w:footnotePr>{inner}</w:footnotePr>"

    each_page = '<w:numRestart w:val="eachPage"/>'
    each_section = '<w:numRestart w:val="eachSect"/>'
    # Numbers that run on through the document.
    write(directory / "notes-continuous.docx", body(), footnotes=notes, settings=of_document(""))
    # Numbers that start again on each page.
    write(directory / "notes-each-page.docx", body(), of_section(each_page), notes, of_document(each_page))
    # Two sections, the second on a new page after line 58. Numbers start
    # again in each; the second has capital Roman numerals from V.
    second = of_section('<w:numFmt w:val="upperRoman"/><w:numStart w:val="5"/>' + each_section)
    write(directory / "notes-each-section.docx", body(58, of_section(each_section)), second, notes, of_document(each_section))
    # The same two sections with numbers that run on; the second says to start from 5.
    write(directory / "notes-section-start.docx", body(58, of_section("")), of_section('<w:numStart w:val="5"/>'), notes, of_document(""))


def grid_foot(directory):
    """How much room the last line of a page needs on a document grid. Thirty
    lines of 10.5 pt Chinese on a grid of 15.6 pt; each takes two rows and
    its text stands in the middle of them. The bottom margin leaves 25.0 pt
    for the 22nd line in one document and 25.5 pt in the other: WPS puts
    the line on the first page in the second only."""
    styles = (
        '<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Liberation Serif" w:hAnsi="Liberation Serif"'
        ' w:eastAsia="Source Han Serif CN" w:cs="Liberation Serif"/><w:sz w:val="21"/><w:szCs w:val="21"/>'
        '<w:lang w:val="en-US" w:eastAsia="zh-CN"/></w:rPr></w:rPrDefault><w:pPrDefault/></w:docDefaults>'
        '<w:style w:type="paragraph" w:styleId="a" w:default="1"><w:name w:val="Normal"/>'
        '<w:pPr><w:widowControl w:val="0"/><w:jc w:val="both"/></w:pPr></w:style>'
    )
    hint = '<w:rPr><w:rFonts w:hint="eastAsia"/></w:rPr>'
    body = "".join(f"<w:p><w:r>{hint}<w:t>第{number:02d}行正文</w:t></w:r></w:p>" for number in range(1, 31))
    # The 22nd line begins 14544 twips down the page.
    for name, room in (("grid-foot-25-0", 500), ("grid-foot-25-5", 510)):
        page = (
            '<w:pgSz w:w="11906" w:h="16838"/>'
            f'<w:pgMar w:top="1440" w:right="1800" w:bottom="{16838 - 14544 - room}" w:left="1800"'
            ' w:header="851" w:footer="992" w:gutter="0"/><w:docGrid w:type="lines" w:linePitch="312"/>'
        )
        write(directory / f"{name}.docx", body, styles=styles, page=page)


def main():
    directory = Path(sys.argv[1] if len(sys.argv) > 1 else "testdata/probes")
    directory.mkdir(parents=True, exist_ok=True)
    paragraph_borders(directory / "paragraph-borders.docx")
    shading_and_tables(directory / "shading-and-tables.docx", cell_margins=False)
    shading_and_tables(directory / "table-lines.docx", cell_margins=True)
    footnotes(directory)
    grid_foot(directory)
    print("\n".join(sorted(path.name for path in directory.glob("*.docx"))))


if __name__ == "__main__":
    main()
