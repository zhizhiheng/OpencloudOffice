#!/usr/bin/env python3
"""Renders documents with LibreOffice to serve as a layout reference.

For each .docx this writes, under out/ref/<name>/:

    document.pdf   LibreOffice's rendering
    page-N.png     one image per page
    lines.json     every text line with its page, position and size

LibreOffice runs with the font mapping in tools/fonts/fontconfig.conf, so it
uses the same substitute fonts as our engine. It is a second opinion on
layout, not ground truth: it differs from Word in places, notably in how it
treats the document grid and character-unit indents.

    python3 tools/zh/reference.py testdata/zh/*.docx
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

import pdfplumber

ROOT = Path(__file__).resolve().parent.parent.parent
OUT = ROOT / "out/ref"


def text_lines(pdf_path):
    """Groups characters into lines by baseline."""
    pages = []
    with pdfplumber.open(pdf_path) as pdf:
        for page in pdf.pages:
            rows = {}
            for ch in page.chars:
                # pdfplumber's `bottom` of a character box is not its baseline;
                # the text matrix origin is. Round to merge sub-point jitter.
                baseline = round(float(page.height) - ch["matrix"][5], 1)
                rows.setdefault(baseline, []).append(ch)
            # Superscripts, subscripts and raised text have baselines of their
            # own. Fold each such fragment into the line it sits beside: the
            # nearest row within half its height that holds more characters.
            height = {b: max(c["size"] for c in chars) for b, chars in rows.items()}
            for baseline in sorted(rows, key=lambda b: len(rows[b])):
                near = [b for b in rows if b != baseline and abs(b - baseline) < 0.6 * height[b] and len(rows[b]) > len(rows[baseline])]
                if near:
                    host = min(near, key=lambda b: abs(b - baseline))
                    rows[host].extend(rows.pop(baseline))
            lines = []
            for baseline in sorted(rows):
                chars = sorted(rows[baseline], key=lambda c: c["x0"])
                text = "".join(c["text"] for c in chars)
                if not text.strip():
                    continue
                lines.append({
                    "baseline": baseline,
                    "x0": round(chars[0]["x0"], 2),
                    "x1": round(chars[-1]["x1"], 2),
                    "size": round(max(c["size"] for c in chars), 2),
                    "text": text,
                })
            pages.append({"width": float(page.width), "height": float(page.height), "lines": lines})
    return pages


def render(docx: Path) -> None:
    target = OUT / docx.stem
    shutil.rmtree(target, ignore_errors=True)
    target.mkdir(parents=True)
    env = dict(os.environ, FONTCONFIG_FILE=str(ROOT / "tools/fonts/fontconfig.conf"))
    with tempfile.TemporaryDirectory() as profile:
        subprocess.run(
            ["soffice", "--headless", f"-env:UserInstallation=file://{profile}", "--convert-to", "pdf",
             "--outdir", str(target), str(docx)],
            check=True, capture_output=True, env=env, cwd=ROOT,
        )
    pdf = target / "document.pdf"
    (target / f"{docx.stem}.pdf").rename(pdf)
    subprocess.run(["pdftoppm", "-png", "-r", "110", str(pdf), str(target / "page")], check=True)
    pages = text_lines(pdf)
    (target / "lines.json").write_text(json.dumps(pages, ensure_ascii=False, indent=1))
    print(f"{docx.name}: {len(pages)} pages, {sum(len(p['lines']) for p in pages)} lines")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    for argument in sys.argv[1:]:
        render(Path(argument).resolve())
