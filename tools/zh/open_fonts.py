#!/usr/bin/env python3
"""Writes a copy of the test documents that names open fonts directly.

The documents in testdata/zh name the fonts Chinese documents really use:
宋体, 黑体, 仿宋, 楷体, Times New Roman, Arial. A machine without them has
to substitute, and applications substitute differently: our engine keeps the
measurements of the font that was asked for, while WPS and LibreOffice take
those of the font they draw with. Comparing layouts then compares fonts.

The copies in testdata/zh-open name instead the open fonts that
tools/fonts/fetch.sh fetches and that most Linux systems carry, so that every
application lays the same text out in the same font, and what differs is the
layout rules. The price is that these are no longer the fonts of real
documents: rules that depend on 宋体's own measurements are not exercised.

    python3 tools/zh/open_fonts.py
"""

import re
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
SOURCE = ROOT / "testdata/zh"
TARGET = ROOT / "testdata/zh-open"

FONTS = {
    "宋体": "Source Han Serif CN",
    "仿宋": "Source Han Serif CN",
    "方正小标宋简体": "Source Han Serif CN",
    "黑体": "Source Han Sans CN",
    "楷体": "LXGW WenKai",
    "Times New Roman": "Liberation Serif",
    "Arial": "Liberation Sans",
}

FONT_ELEMENT = re.compile(r"<w:rFonts\b[^>]*>")
FONT_ATTRIBUTE = re.compile(r'(w:(?:ascii|hAnsi|eastAsia|cs))="([^"]*)"')


def rewrite(xml: str, unknown: set) -> str:
    def attribute(match):
        name = match.group(2)
        if name not in FONTS:
            unknown.add(name)
        return f'{match.group(1)}="{FONTS.get(name, name)}"'

    return FONT_ELEMENT.sub(lambda element: FONT_ATTRIBUTE.sub(attribute, element.group(0)), xml)


def main():
    TARGET.mkdir(exist_ok=True)
    unknown = set()
    for source in sorted(SOURCE.glob("*.docx")):
        with zipfile.ZipFile(source) as package, zipfile.ZipFile(TARGET / source.name, "w") as out:
            for item in package.infolist():
                data = package.read(item)
                if item.filename.endswith(".xml"):
                    data = rewrite(data.decode("utf-8"), unknown).encode("utf-8")
                # A fixed date, so that the same input gives the same bytes.
                entry = zipfile.ZipInfo(item.filename, date_time=(2026, 1, 1, 0, 0, 0))
                entry.compress_type = zipfile.ZIP_DEFLATED
                out.writestr(entry, data)
        print(f"{source.name} -> {(TARGET / source.name).relative_to(ROOT)}")
    if unknown:
        print("fonts left as they were (add them to FONTS): " + ", ".join(sorted(unknown)))


if __name__ == "__main__":
    main()
