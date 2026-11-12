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

# How far, in points, a line may be from the reference's and still count as
# being in the same place. Half a point is a sixth of a millimetre.
NEAR = 0.5


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


# A reference layout is kept either as the PDF itself or, where PDFs are too
# big to commit, as the lines read from it: `<name>.lines.json`.
LINES_SUFFIX = ".lines.json"


def reference_files(directory: Path):
    """The reference layouts in `directory`, by document path without extension.
    Where both a PDF and its lines file exist, the PDF is used."""
    found = {}
    if directory.is_dir():
        for file in sorted(directory.rglob("*" + LINES_SUFFIX)):
            found[file.relative_to(directory).as_posix()[: -len(LINES_SUFFIX)]] = file
        for file in sorted(directory.rglob("*.pdf")):
            found[file.relative_to(directory).with_suffix("").as_posix()] = file
    return found


def reference_lines(file: Path):
    """The lines of a reference layout, from a PDF or from a lines file."""
    if file.name.endswith(LINES_SUFFIX):
        return json.loads(file.read_text(encoding="utf-8"))
    return pdf_lines(file)


def write_lines_file(pdf: Path):
    """Writes the lines of `pdf` beside it, for committing in the PDF's place."""
    target = pdf.with_name(pdf.stem + LINES_SUFFIX)
    target.write_text(json.dumps(pdf_lines(pdf), ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    return target


def our_lines(docx: Path, out: Path, model: str = "word"):
    """Lays the document out with our engine into `out` and returns its lines."""
    subprocess.run([str(OCO), "render", str(docx), "--out", str(out), "--model", model],
                   check=True, capture_output=True, cwd=ROOT)
    return json.loads((out / "lines.json").read_text(encoding="utf-8"))


def our_summary(out: Path):
    """What the engine said about the document it laid out into `out`: the
    fonts it lacked, and whether the document holds content it does not lay out."""
    return json.loads((out / "layout.json").read_text(encoding="utf-8"))


def key(text):
    """What must match for two lines to count as the same line."""
    # Tab leaders are text in a PDF and a drawn rule in ours.
    return re.sub(r"[.\-_·…]{4,}", "", re.sub(r"\s", "", text)).rstrip(TRAILING_MARKS)


def across(page):
    """Joins the lines of a page that share a baseline, as the cells of a
    table row do, into one line read from left to right. That is how they
    come out of a PDF, where nothing says which cell a character is in."""
    rows = []
    for line in sorted(page["lines"], key=lambda l: (l["baseline"], l["x0"])):
        if rows and abs(line["baseline"] - rows[-1][0]["baseline"]) < 0.06:
            rows[-1].append(line)
        else:
            rows.append([line])
    lines = []
    for row in rows:
        row.sort(key=lambda l: l["x0"])
        lines.append({
            "baseline": row[0]["baseline"],
            "x0": row[0]["x0"],
            "x1": max(l["x1"] for l in row),
            "text": " ".join(l["text"] for l in row),
        })
    return {**page, "lines": lines}


def first_body_line(page, body):
    """Index of the first line of `page` that is not in its header: the first
    below the top of the body area, or the very first when none is."""
    if body:
        for index, line in enumerate(page["lines"]):
            if line["baseline"] > body[0]:
                return index
    return 0


def compare(ours, reference):
    """Compares our layout with a reference layout of the same document.

    A page starts with its first line of body text; a header above it does
    not count. Our own pages say where the body area is, and the reference's
    pages are taken to have the same margins.

    Returns the reference's line count; how many of those lines we break
    identically (`same`); how many of them also sit where the reference has
    them, on the same page and within `NEAR` points both ways (`placed`);
    the page counts; how many of the reference's page starts are page starts
    of ours; and the median distance between the lines that match: `dx`
    between their left ends, `dy` between their baselines when they are on
    the same page.
    """
    ours = [across(page) for page in ours]
    flat = lambda pages: [(number, index, line) for number, page in enumerate(pages) for index, line in enumerate(page["lines"])]
    a, b = flat(ours), flat(reference)
    bodies = [page.get("body") for page in ours]
    body_of = lambda number: bodies[min(number, len(bodies) - 1)] if bodies else None
    first_of_ours = [first_body_line(page, page.get("body")) for page in ours]
    first_of_reference = [first_body_line(page, body_of(number)) for number, page in enumerate(reference)]
    matcher = difflib.SequenceMatcher(None, [key(e[2]["text"]) for e in a], [key(e[2]["text"]) for e in b], autojunk=False)
    same, placed, dx, dy, starts_equal = 0, 0, [], [], 0
    for tag, i1, i2, j1, j2 in matcher.get_opcodes():
        if tag != "equal":
            continue
        for i, j in zip(range(i1, i2), range(j1, j2)):
            same += 1
            (page_a, index_a, line_a), (page_b, index_b, line_b) = a[i], b[j]
            dx.append(abs(line_a["x0"] - line_b["x0"]))
            if page_a == page_b:
                dy.append(abs(line_a["baseline"] - line_b["baseline"]))
                placed += dx[-1] <= NEAR and dy[-1] <= NEAR
            # A page of the reference begins with this line, and so does one of ours.
            if page_b > 0 and index_b == first_of_reference[page_b] and page_a > 0 and index_a == first_of_ours[page_a]:
                starts_equal += 1
    median = lambda values: round(statistics.median(values), 2) if values else None
    return {
        "lines": len(b),
        "same": same,
        "placed": placed,
        "pages": [len(reference), len(ours)],
        "page_starts": sum(1 for page in reference[1:] if page["lines"]),
        "page_starts_equal": starts_equal,
        "dx": median(dx),
        "dy": median(dy),
    }
