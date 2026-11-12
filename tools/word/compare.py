#!/usr/bin/env python3
"""Compares our layout with a reference application's own, line by line.

The reference is a set of PDFs that the application exported from the
documents. For a folder of documents they live in a subfolder named after
the application, one PDF per document, under the same name:

    testdata/zh/04-report-headings.docx
    testdata/zh/word/04-report-headings.pdf    exported by Word
    testdata/zh/wps/04-report-headings.pdf     exported by WPS Office

Where the PDFs are too big to commit, the lines read from each stand in for
it: `04-report-headings.lines.json`, which the export scripts write.

Word is the standard the engine follows. WPS is a second reference: it lays
Chinese documents out much as Word does, and is easier to come by.
tools/word/export.ps1 makes Word's PDFs, tools/wps/export.py those of WPS.

For each pair this lays the document out with our engine (following Word)
and reports: the page counts; how many of the reference's lines we break at
the same places; how many of those also sit within half a point of where
the reference has them; how many of its pages begin with the line ours
begins with; and the median distance of the matching lines, across (dx) and
down (dy).

    cargo build --release
    python3 tools/word/compare.py [testdata/zh] [--reference word|wps] [--json FILE]
"""

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent

sys.path.insert(0, str(ROOT / "tools"))
from common.layout_lines import OCO, compare, our_lines, our_summary, reference_files, reference_lines  # noqa: E402

# Narrowing groups of documents, as in `oco score`: the last holds those the
# engine has no excuse for getting wrong.
GROUPS = [
    ("all", "all documents"),
    ("plain", "... with only body text"),
    ("covered", "... and every font available"),
]


def pairs(documents: Path, reference: Path):
    """(document, the reference's layout of it) for every reference that has its document."""
    found = []
    for stem, file in reference_files(reference).items():
        for extension in (".docx", ".docm"):
            docx = documents / (stem + extension)
            if docx.exists():
                found.append((docx, file))
                break
    return found


def measure(documents: Path, reference: Path, out: Path):
    """Results by document path relative to `documents`."""
    results = {}
    for docx, file in pairs(documents, reference):
        target = out / docx.relative_to(documents).with_suffix("")
        result = compare(our_lines(docx, target), reference_lines(file))
        summary = our_summary(target)
        result["plain"] = not summary["beyond_body_text"]
        result["missing_fonts"] = summary["missing_fonts"]
        results[docx.relative_to(documents).as_posix()] = result
    return results


def member(result, group):
    return group == "all" or result["plain"] and (group == "plain" or not result["missing_fonts"])


def totals(results):
    """The figures of each group, summed over its documents."""
    groups = {}
    for name, _ in GROUPS:
        members = [r for r in results.values() if member(r, name)]
        total = lambda field: sum(r[field] for r in members)
        groups[name] = {
            "documents": len(members),
            "pages_equal": sum(r["pages"][0] == r["pages"][1] for r in members),
            "lines": total("lines"),
            "same": total("same"),
            "placed": total("placed"),
            "page_starts": total("page_starts"),
            "page_starts_equal": total("page_starts_equal"),
        }
    return groups


def table(results):
    lines = [
        f"{'document':34s} {'pages':>10s} {'lines':>6s} {'same break':>16s} {'in place':>9s} {'page starts':>12s} {'dx':>6s} {'dy':>6s}",
        f"{'':34s} {'ref. ours':>10s} {'':6s} {'':16s} {'':9s} {'same':>12s} {'pt':>6s} {'pt':>6s}",
    ]
    distance = lambda value: "     -" if value is None else f"{value:6.1f}"
    for name, r in results.items():
        lines.append(
            f"{name[-34:]:34s} {r['pages'][0]:5d} {r['pages'][1]:4d} {r['lines']:6d} "
            f"{r['same']:6d} {100 * r['same'] / max(1, r['lines']):8.1f}% {r['placed']:9d} "
            f"{r['page_starts_equal']:6d}/{r['page_starts']:<5d} {distance(r['dx'])} {distance(r['dy'])}"
        )
    share = lambda count, total: f"{count}/{total} ({100 * count / max(1, total):.1f}%)"
    lines.append("")
    for name, title in GROUPS:
        g = totals(results)[name]
        lines.append(
            f"{title:30s} {g['documents']:4d} documents; same page count {g['pages_equal']}; "
            f"lines broken the same {share(g['same'], g['lines'])}; in place {share(g['placed'], g['lines'])}; "
            f"page starts {share(g['page_starts_equal'], g['page_starts'])}"
        )
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("documents", nargs="?", default="testdata/zh", help="folder of documents (default: testdata/zh)")
    parser.add_argument("--reference", default="word", help="whose PDFs to compare with: word (default) or wps")
    parser.add_argument("--json", metavar="FILE", help="also write the result of each document")
    args = parser.parse_args()
    documents = (ROOT / args.documents).resolve()
    reference = documents / args.reference
    if not OCO.exists():
        sys.exit(f"{OCO} not found; run `cargo build --release` first")
    if not pairs(documents, reference):
        sys.exit(
            f"no reference PDFs (or lines files) in {reference}\n"
            "Word's are made on a Windows machine that has Word:\n"
            f"    powershell -ExecutionPolicy Bypass -File tools\\word\\export.ps1 -In {args.documents}\n"
            "those of WPS Office wherever WPS is installed:\n"
            f"    python3 tools/wps/export.py {args.documents}"
        )
    results = measure(documents, reference, ROOT / "out" / f"{args.reference}-compare" / documents.name)
    print(table(results))
    if args.json:
        Path(args.json).write_text(json.dumps(results, ensure_ascii=False, indent=1), encoding="utf-8")


if __name__ == "__main__":
    main()
