"""Text lines of a laid-out document, from a PDF or from our own engine, and
how to compare two such layouts line by line.

A layout is a list of pages; a page is {"width", "height", "lines"}; a line
is {"baseline", "x0", "x1", "text"}, in points from the page's top left.
"""

import difflib
import json
import re
import statistics
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
OCO = ROOT / "target/release/oco"

# A closing mark hung in the margin can be missing from a PDF's text, so
# trailing marks are left out when lines are compared.
TRAILING_MARKS = "，。、；：？！）”’》】"


def pdf_lines(pdf_path):
    """Reads a PDF and groups its characters into lines by baseline."""
    import pdfplumber

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


def our_lines(docx: Path, out: Path, model: str = "word"):
    """Lays the document out with our engine into `out` and returns its lines."""
    subprocess.run([str(OCO), "render", str(docx), "--out", str(out), "--model", model],
                   check=True, capture_output=True, cwd=ROOT)
    return json.loads((out / "lines.json").read_text(encoding="utf-8"))


def key(text):
    """What must match for two lines to count as the same line."""
    # Dot leaders are text in a PDF and a drawn rule in ours.
    return re.sub(r"\.{4,}", "", re.sub(r"\s", "", text)).rstrip(TRAILING_MARKS)


def compare(ours, reference):
    """Compares our layout with a reference layout of the same document.

    Returns the reference's line count, how many of those lines we break
    identically, the page counts, how many of the reference's page starts
    are page starts of ours, and the median distance between the lines that
    match: `dx` between their left ends, `dy` between their baselines when
    they are on the same page.
    """
    flat = lambda pages: [(number, index, line) for number, page in enumerate(pages) for index, line in enumerate(page["lines"])]
    a, b = flat(ours), flat(reference)
    matcher = difflib.SequenceMatcher(None, [key(e[2]["text"]) for e in a], [key(e[2]["text"]) for e in b], autojunk=False)
    same, dx, dy, starts_equal = 0, [], [], 0
    for tag, i1, i2, j1, j2 in matcher.get_opcodes():
        if tag != "equal":
            continue
        for i, j in zip(range(i1, i2), range(j1, j2)):
            same += 1
            (page_a, index_a, line_a), (page_b, index_b, line_b) = a[i], b[j]
            dx.append(abs(line_a["x0"] - line_b["x0"]))
            if page_a == page_b:
                dy.append(abs(line_a["baseline"] - line_b["baseline"]))
            # A page of the reference begins with this line, and so does one of ours.
            if page_b > 0 and index_b == 0 and page_a > 0 and index_a == 0:
                starts_equal += 1
    median = lambda values: round(statistics.median(values), 2) if values else None
    return {
        "lines": len(b),
        "same": same,
        "pages": [len(reference), len(ours)],
        "page_starts": sum(1 for page in reference[1:] if page["lines"]),
        "page_starts_equal": starts_equal,
        "dx": median(dx),
        "dy": median(dy),
    }
