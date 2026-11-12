#!/usr/bin/env python3
"""Surveys a corpus of .docx files for facts that inform design decisions.

Unlike `oco corpus`, this does not use our reader. It looks at the raw
packages and answers questions such as:

  - How much of a file must be downloaded before text can be shown?
  - How many files carry Word's own page count or page-break hints, which can
    serve as layout ground truth without running Word?
  - How common are the features we have not built yet?

    python3 tools/corpus/survey.py corpus [--json survey.json]
"""

import argparse
import json
import re
import statistics
import sys
import zipfile
from collections import Counter
from pathlib import Path

EXTENSIONS = {".docx", ".docm", ".dotx", ".dotm"}

# Parts needed to lay out the body text of the first page.
CORE = re.compile(
    r"^(\[content_types\]\.xml|_rels/\.rels|word/(_rels/[^/]+\.rels|document\d*\.xml|styles\.xml|numbering\.xml"
    r"|settings\.xml|fonttable\.xml|websettings\.xml|theme/[^/]+\.xml))$"
)
# Text outside the body; needed for a complete page but not for the first paint.
SECONDARY = re.compile(r"^word/(header|footer|footnotes|endnotes|comments)[^/]*\.xml$")

FEATURES = {
    "tables": rb"<w:tbl[ >]",
    "table style applied": rb"<w:tblStyle ",
    "lists (numbering)": rb"<w:numPr>",
    "images (DrawingML)": rb"<w:drawing>",
    "images or shapes (VML)": rb"<w:pict>",
    "text boxes": rb"<w:txbxContent>",
    "fields": rb"<w:fldChar |<w:fldSimple ",
    "hyperlinks": rb"<w:hyperlink ",
    "tracked insert/delete": rb"<w:(ins|del|moveFrom|moveTo) ",
    "tracked formatting change": rb"<w:(rPrChange|pPrChange|tblPrChange|tcPrChange|sectPrChange) ",
    "comments": rb"<w:commentReference ",
    "footnotes/endnotes": rb"<w:(footnoteReference|endnoteReference) ",
    "content controls": rb"<w:sdt>",
    "math": rb"<m:oMath[ >]",
    "multiple sections": rb"<w:sectPr.*<w:sectPr",
    "multiple columns": rb'<w:cols [^>]*w:num="[2-9]',
    "tab stops": rb"<w:tabs>",
    "paragraph borders/shading": rb"<w:pBdr>|<w:pPr>(?:(?!</w:pPr>).)*<w:shd ",
    "embedded objects": rb"<w:object[ >]",
    "charts": rb"<c:chart ",
    "SmartArt": rb"<dgm:relIds ",
    "frames (framePr)": rb"<w:framePr ",
    "character-unit indents": rb'w:(firstLineChars|hangingChars|leftChars|rightChars|startChars|endChars)="',
    "line-unit spacing": rb'w:(beforeLines|afterLines)="',
    "document grid (lines)": rb'<w:docGrid [^>]*w:type="(lines|linesAndChars|snapToChars)"',
    "East Asian spacing options": rb"<w:(autoSpaceDE|autoSpaceDN|kinsoku|overflowPunct|topLinePunct|snapToGrid)[ /]",
    "bidirectional text": rb"<w:(bidi|rtl)[ /]",
    "Word page-break hints": rb"<w:lastRenderedPageBreak/>",
    "compatibility fallback (mc)": rb"<mc:AlternateContent",
}
FEATURE_PATTERNS = {name: re.compile(pattern, re.S) for name, pattern in FEATURES.items()}
TEXT = re.compile(rb"<w:t(?: [^>]*)?>([^<]*)</w:t>")
CJK = re.compile(r"[぀-ヿ㐀-䶿一-鿿가-힯豈-﫿]")


def app_property(xml: bytes, name: str):
    m = re.search(rb"<" + name.encode() + rb">([^<]*)</" + name.encode() + rb">", xml)
    return m.group(1).decode("utf-8", "replace") if m else None


def survey(path: Path):
    try:
        archive = zipfile.ZipFile(path)
        infos = archive.infolist()
        names = {i.filename.lower().lstrip("/"): i for i in infos}
        rels = archive.read(names["_rels/.rels"].filename) if "_rels/.rels" in names else b""
        m = re.search(rb'Type="[^"]*/officeDocument"[^>]*Target="/?([^"]+)"', rels) or re.search(
            rb'Target="/?([^"]+)"[^>]*Type="[^"]*/officeDocument"', rels
        )
        main = (m.group(1).decode() if m else "word/document.xml").lower()
        if main not in names:
            return None
        document = archive.read(names[main].filename)
        app = archive.read(names["docprops/app.xml"].filename) if "docprops/app.xml" in names else b""
    except Exception:
        return None

    sizes = Counter()
    for info in infos:
        name = info.filename.lower().lstrip("/")
        if CORE.match(name):
            kind = "core"
        elif SECONDARY.match(name):
            kind = "secondary"
        elif name.startswith("word/media/"):
            kind = "media"
        elif name.startswith("word/fonts/"):
            kind = "fonts"
        elif name.startswith(("word/embeddings/", "word/activex/")):
            kind = "embedded"
        else:
            kind = "other"
        # 30 bytes of local header plus the name precede each entry's data.
        sizes[kind] += info.compress_size + 30 + len(info.filename)

    text = b"".join(TEXT.findall(document)).decode("utf-8", "replace")
    pages = app_property(app, "Pages")
    return {
        "path": str(path),
        "bytes": path.stat().st_size,
        "sizes": dict(sizes),
        "document_xml_bytes": len(document),
        "application": app_property(app, "Application"),
        "pages": int(pages) if pages and pages.isdigit() else None,
        "lines": app_property(app, "Lines"),
        "text_chars": len(text),
        "cjk_chars": len(CJK.findall(text)),
        "page_break_hints": len(FEATURE_PATTERNS["Word page-break hints"].findall(document)),
        "features": [name for name, pattern in FEATURE_PATTERNS.items() if pattern.search(document)],
    }


def percent(n, of):
    return f"{100 * n / of:5.1f}%" if of else "    -"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("roots", nargs="+")
    parser.add_argument("--json", help="write the per-file records here")
    args = parser.parse_args()

    paths = sorted(
        p for root in args.roots for p in Path(root).rglob("*")
        if p.suffix.lower() in EXTENSIONS and ".git" not in p.parts and p.is_file()
    )
    records = [r for r in map(survey, paths) if r]
    n = len(records)
    print(f"{len(paths)} files, {n} readable packages\n")

    def producer(r):
        app = (r["application"] or "unknown").lower()
        for key, label in (("microsoft", "Microsoft Word"), ("libreoffice", "LibreOffice"), ("openoffice", "OpenOffice"),
                           ("wps", "WPS"), ("docx4j", "docx4j"), ("onlyoffice", "ONLYOFFICE"), ("unknown", "(not stated)")):
            if key in app:
                return label
        return "other"

    print("saved by")
    for label, count in Counter(map(producer, records)).most_common():
        print(f"  {count:5d} {percent(count, n)}  {label}")

    cjk = [r for r in records if r["text_chars"] and r["cjk_chars"] / r["text_chars"] > 0.2]
    print(f"\nmostly East Asian text: {len(cjk)} ({percent(len(cjk), n).strip()})")

    print("\nfeatures, by documents using them        all docs   East Asian docs")
    counts = Counter(f for r in records for f in r["features"])
    cjk_counts = Counter(f for r in cjk for f in r["features"])
    for name, count in counts.most_common():
        print(f"  {name:36s} {count:5d} {percent(count, n)}   {percent(cjk_counts[name], len(cjk))}")

    with_pages = [r for r in records if r["pages"]]
    with_hints = [r for r in records if r["page_break_hints"]]
    multi = [r for r in with_pages if r["pages"] > 1]
    consistent = [r for r in multi if r["page_break_hints"] == r["pages"] - 1]
    print("\nlayout ground truth already inside the files")
    print(f"  page count recorded by the producer   {len(with_pages):5d} {percent(len(with_pages), n)}")
    print(f"  page-break hints from Word            {len(with_hints):5d} {percent(len(with_hints), n)}")
    print(f"  multi-page, hints == pages - 1        {len(consistent):5d} of {len(multi)} multi-page files")

    def fraction(r):
        return r["sizes"].get("core", 0) / max(1, sum(r["sizes"].values()))

    print("\nshare of the file needed to show the first page of text (core parts / total)")
    for label, low, high in (("< 100 KB", 0, 1e5), ("100 KB - 1 MB", 1e5, 1e6), ("> 1 MB", 1e6, 1e18)):
        group = [r for r in records if low <= r["bytes"] < high]
        if not group:
            continue
        fractions = sorted(map(fraction, group))
        core = statistics.median(r["sizes"].get("core", 0) for r in group)
        print(f"  {label:14s} {len(group):5d} files  median share {100 * statistics.median(fractions):5.1f}%"
              f"  median core size {core / 1024:7.1f} KB")
    total = Counter()
    for r in records:
        total.update(r["sizes"])
    whole = sum(total.values())
    print("  bytes by kind across the corpus: " + ", ".join(f"{k} {100 * v / whole:.1f}%" for k, v in total.most_common()))
    embedded_fonts = sum(1 for r in records if r["sizes"].get("fonts"))
    print(f"  files embedding fonts: {embedded_fonts} ({percent(embedded_fonts, n).strip()})")

    if args.json:
        Path(args.json).write_text(json.dumps(records, ensure_ascii=False))


if __name__ == "__main__":
    sys.exit(main())
