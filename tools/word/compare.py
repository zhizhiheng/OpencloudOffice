#!/usr/bin/env python3
"""Compares our layout with Word's own, line by line.

Word's side is a set of PDFs that Word exported from the documents (see
tools/word/export.ps1, which makes them on a Windows machine with Word).
For a folder of documents they live in its `word` subfolder, one PDF per
document, under the same name:

    testdata/zh/04-report-headings.docx
    testdata/zh/word/04-report-headings.pdf
    testdata/zh/word/manifest.json          which Word made them, and how

For each pair this lays the document out with our engine (following Word)
and reports: the page counts; how many of Word's lines we break at the same
places; how many of Word's pages begin with the line ours begins with; and
how far the matching lines are from Word's, across (dx) and down (dy).

    cargo build --release
    python3 tools/word/compare.py [testdata/zh] [--word DIR] [--json FILE]
"""

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent

sys.path.insert(0, str(ROOT / "tools"))
from common.layout_lines import OCO, compare, our_lines, pdf_lines  # noqa: E402


def pairs(documents: Path, word: Path):
    """(document, Word's PDF of it) for every PDF that has its document."""
    found = []
    for pdf in sorted(word.rglob("*.pdf")):
        stem = pdf.relative_to(word).with_suffix("")
        for extension in (".docx", ".docm"):
            docx = documents / stem.with_suffix(extension)
            if docx.exists():
                found.append((docx, pdf))
                break
    return found


def measure(documents: Path, word: Path, out: Path):
    """Results by document path relative to `documents`."""
    results = {}
    for docx, pdf in pairs(documents, word):
        name = docx.relative_to(documents).as_posix()
        results[name] = compare(our_lines(docx, out / docx.relative_to(documents).with_suffix("")), pdf_lines(pdf))
    return results


def table(results):
    lines = [
        f"{'document':34s} {'pages':>10s} {'lines':>6s} {'same break':>16s} {'page starts':>12s} {'dx':>6s} {'dy':>6s}",
        f"{'':34s} {'Word ours':>10s} {'':6s} {'':16s} {'same':>12s} {'pt':>6s} {'pt':>6s}",
    ]
    distance = lambda value: "     -" if value is None else f"{value:6.1f}"
    for name, r in results.items():
        lines.append(
            f"{name[-34:]:34s} {r['pages'][0]:5d} {r['pages'][1]:4d} {r['lines']:6d} "
            f"{r['same']:6d} {100 * r['same'] / max(1, r['lines']):8.1f}% "
            f"{r['page_starts_equal']:6d}/{r['page_starts']:<5d} {distance(r['dx'])} {distance(r['dy'])}"
        )
    total = lambda field: sum(r[field] for r in results.values())
    lines.append("")
    lines.append(
        f"{len(results)} documents; page count as Word's in {sum(r['pages'][0] == r['pages'][1] for r in results.values())}; "
        f"lines broken as Word's: {total('same')} of {total('lines')} ({100 * total('same') / max(1, total('lines')):.1f}%); "
        f"page starts as Word's: {total('page_starts_equal')} of {total('page_starts')}"
    )
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("documents", nargs="?", default="testdata/zh", help="folder of documents (default: testdata/zh)")
    parser.add_argument("--word", metavar="DIR", help="folder of Word's PDFs (default: <documents>/word)")
    parser.add_argument("--json", metavar="FILE", help="also write the result of each document")
    args = parser.parse_args()
    documents = (ROOT / args.documents).resolve()
    word = (ROOT / args.word).resolve() if args.word else documents / "word"
    if not OCO.exists():
        sys.exit(f"{OCO} not found; run `cargo build --release` first")
    if not pairs(documents, word):
        sys.exit(
            f"no Word PDFs in {word}\n"
            "Make them on a Windows machine that has Word:\n"
            f"    powershell -ExecutionPolicy Bypass -File tools\\word\\export.ps1 -In {args.documents}\n"
            "and copy the resulting `word` folder here."
        )
    results = measure(documents, word, ROOT / "out/word-compare" / documents.name)
    print(table(results))
    if args.json:
        Path(args.json).write_text(json.dumps(results, ensure_ascii=False, indent=1), encoding="utf-8")


if __name__ == "__main__":
    main()
