#!/usr/bin/env python3
"""Compares our layout of the test documents with LibreOffice's.

Renders every document under testdata/zh with `oco render`, twice:

  out/ours/<name>              following Word (what a product would use)
  out/ours-libreoffice/<name>  following LibreOffice where the two differ

and compares the line breaks, line positions and page counts of the second
with the LibreOffice reference in out/ref (write it with tools/zh/reference.py).
The first is the engine's real output; it is expected to differ from
LibreOffice wherever Word does, most visibly in the page counts.

    cargo build --release
    python3 tools/zh/reference.py testdata/zh/*.docx
    python3 tools/zh/compare.py [--png]
"""

import difflib
import json
import re
import statistics
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
OCO = ROOT / "target/release/oco"
SKIP = re.compile(r"\s")


def lines_of(directory: Path):
    pages = json.loads((directory / "lines.json").read_text())
    return [(index, line) for index, page in enumerate(pages) for line in page["lines"]], len(pages)


def render(docx: Path, out: Path, model: str) -> None:
    subprocess.run([str(OCO), "render", str(docx), "--out", str(out), "--model", model],
                   check=True, capture_output=True, cwd=ROOT)


def main() -> None:
    documents = sorted((ROOT / "testdata/zh").glob("*.docx"))
    print(f"{'document':28s} {'pages':>17s}   {'lines':>5s} {'same break':>10s}   {'dx':>5s} {'dy':>5s}")
    print(f"{'':28s} {'LO  as-LO  ours':>17s}   {'':5s} {'':10s}   {'pt':>5s} {'pt':>5s}")
    totals = [0, 0]
    for docx in documents:
        name = docx.stem
        reference_dir = ROOT / "out/ref" / name
        if not (reference_dir / "lines.json").exists():
            print(f"{name:28s} no reference; run tools/zh/reference.py")
            continue
        render(docx, ROOT / "out/ours" / name, "word")
        render(docx, ROOT / "out/ours-libreoffice" / name, "libreoffice")
        reference, reference_pages = lines_of(reference_dir)
        ours, our_pages = lines_of(ROOT / "out/ours-libreoffice" / name)
        _, word_pages = lines_of(ROOT / "out/ours" / name)

        # A closing mark hung in the margin is missing from the text of
        # LibreOffice's PDF, so trailing marks are left out of the comparison.
        # Dot leaders are text in the PDF and a drawn rule in ours.
        key = lambda entry: re.sub(r"\.{4,}", "", SKIP.sub("", entry[1]["text"])).rstrip("，。、；：？！）”’》】")
        a, b = [key(e) for e in ours], [key(e) for e in reference]
        same, dx, dy = 0, [], []
        for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes():
            if tag != "equal":
                continue
            for i, j in zip(range(i1, i2), range(j1, j2)):
                same += 1
                (page_a, line_a), (page_b, line_b) = ours[i], reference[j]
                dx.append(abs(line_a["x0"] - line_b["x0"]))
                if page_a == page_b:
                    dy.append(abs(line_a["baseline"] - line_b["baseline"]))
        totals[0] += same
        totals[1] += len(b)
        median = lambda values: f"{statistics.median(values):5.1f}" if values else "    -"
        print(f"{name:28s} {reference_pages:5d} {our_pages:4d} {word_pages:5d}   {len(b):5d} {100 * same / len(b):9.1f}%   {median(dx)} {median(dy)}")
    print(f"\nline breaks identical to LibreOffice's: {totals[0]} of {totals[1]} ({100 * totals[0] / totals[1]:.1f}%)")

    if "--png" in sys.argv:
        pages = sorted(str(p) for kind in ("ours", "ours-libreoffice") for p in (ROOT / "out" / kind).glob("*/page-*.svg"))
        subprocess.run(["node", str(ROOT / "tools/zh/rasterize.mjs"), *pages], check=True, cwd=ROOT)
        print(f"rasterised {len(pages)} pages")


if __name__ == "__main__":
    main()
