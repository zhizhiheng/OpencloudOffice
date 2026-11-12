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

ROOT = Path(__file__).resolve().parent.parent.parent
OUT = ROOT / "out/ref"

sys.path.insert(0, str(ROOT / "tools"))
from common.layout_lines import pdf_lines  # noqa: E402


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
    pages = pdf_lines(pdf)
    (target / "lines.json").write_text(json.dumps(pages, ensure_ascii=False, indent=1))
    print(f"{docx.name}: {len(pages)} pages, {sum(len(p['lines']) for p in pages)} lines")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    for argument in sys.argv[1:]:
        render(Path(argument).resolve())
