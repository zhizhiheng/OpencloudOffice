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

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent

sys.path.insert(0, str(ROOT / "tools"))
from common.layout_lines import compare, our_lines  # noqa: E402


def main() -> None:
    documents = sorted((ROOT / "testdata/zh").glob("*.docx"))
    print(f"{'document':28s} {'pages':>17s}   {'lines':>5s} {'same break':>10s}   {'dx':>5s} {'dy':>5s}")
    print(f"{'':28s} {'LO  as-LO  ours':>17s}   {'':5s} {'':10s}   {'pt':>5s} {'pt':>5s}")
    totals = [0, 0]
    for docx in documents:
        name = docx.stem
        reference_file = ROOT / "out/ref" / name / "lines.json"
        if not reference_file.exists():
            print(f"{name:28s} no reference; run tools/zh/reference.py")
            continue
        reference = json.loads(reference_file.read_text(encoding="utf-8"))
        word_pages = len(our_lines(docx, ROOT / "out/ours" / name, "word"))
        result = compare(our_lines(docx, ROOT / "out/ours-libreoffice" / name, "libreoffice"), reference)
        totals[0] += result["same"]
        totals[1] += result["lines"]
        distance = lambda value: "    -" if value is None else f"{value:5.1f}"
        print(
            f"{name:28s} {result['pages'][0]:5d} {result['pages'][1]:4d} {word_pages:5d}   {result['lines']:5d} "
            f"{100 * result['same'] / result['lines']:9.1f}%   {distance(result['dx'])} {distance(result['dy'])}"
        )
    print(f"\nline breaks identical to LibreOffice's: {totals[0]} of {totals[1]} ({100 * totals[0] / totals[1]:.1f}%)")

    if "--png" in sys.argv:
        pages = sorted(str(p) for kind in ("ours", "ours-libreoffice") for p in (ROOT / "out" / kind).glob("*/page-*.svg"))
        subprocess.run(["node", str(ROOT / "tools/zh/rasterize.mjs"), *pages], check=True, cwd=ROOT)
        print(f"rasterised {len(pages)} pages")


if __name__ == "__main__":
    main()
