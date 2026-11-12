#!/usr/bin/env python3
"""Diffs our reader against LibreOffice, document by document.

Inputs are the reference dumps written by lo_dump.py and the `oco` binary.
Paragraphs are aligned by their text; for every aligned paragraph the
effective formatting is compared character by character. Differences are
grouped by attribute and by where the paragraph sits (plain body text, a list,
a table), because those have different style machinery behind them.

    cargo build --release
    python3 tools/corpus/compare.py corpus-lo [--json diff.json] [--examples 5]

A difference is not automatically our bug: LibreOffice has import quirks of its
own. The output is a ranked list of places worth reading the XML for.
"""

import argparse
import difflib
import json
import re
import subprocess
import sys
from collections import Counter, defaultdict
from multiprocessing import Pool
from pathlib import Path

OCO = Path(__file__).resolve().parent.parent.parent / "target/release/oco"
SKIP = re.compile(r"[\s­​-‏﻿]")  # whitespace and invisible characters
CJK = re.compile(r"[⺀-鿿가-힯豈-﫿＀-￯]")
ADJUST = {None: 0, "left": 0, "start": 0, "right": 1, "end": 1, "both": 2, "distribute": 2, "center": 3}
HUNDREDTH_MM_PER_TWIP = 2540 / 1440


def key(text: str) -> str:
    return SKIP.sub("", text)


def flatten(blocks, in_table, out):
    for block in blocks:
        if block["type"] == "paragraph":
            block["in_table"] = in_table
            out.append(block)
        else:
            for row in block["rows"]:
                for cell in row:
                    flatten(cell, True, out)


def our_chars(paragraph):
    """One (char, run properties) pair per visible character."""
    return [(ch, run["props"]) for run in paragraph["runs"] for ch in run["text"] if not SKIP.match(ch)]


def their_chars(paragraph):
    return [(ch, portion) for portion in paragraph["portions"] for ch in portion["t"] if not SKIP.match(ch)]


def font_name(ref):
    return ref.get("name") if ref else None


def char_checks(ch, ours, theirs):
    """Yields (attribute, our value, their value) for one character."""
    yield "bold", ours.get("bold") is True, theirs["weight"] > 110
    yield "italic", ours.get("italic") is True, theirs["italic"]
    yield "size", ours.get("size", 20) / 2, round(theirs["size"] * 2) / 2
    yield "strike", ours.get("strike") is True, theirs["strike"] == 1
    yield "underline", ours.get("underline") not in (None, "none"), theirs["underline"] != 0
    yield "caps", ours.get("caps") is True, theirs["case"] == 1
    yield "small-caps", ours.get("small_caps") is True and ours.get("caps") is not True, theirs["case"] == 4
    yield "hidden", ours.get("vanish") is True, theirs["hidden"]
    if "\uf000" <= ch <= "\uf0ff":
        return  # a symbol character carries its own font, which `oco inspect` does not print
    fonts = ours.get("fonts", {})
    slot, reference = ("east_asia", theirs["font_asian"]) if CJK.match(ch) else ("ascii", theirs["font"])
    name = font_name(fonts.get(slot))
    if name is not None:  # we only know the font when the document names one
        # LibreOffice writes fallback lists ("Arial;Helvetica") into a single name.
        first = lambda value: value.split(";")[0].strip().lower()
        yield "font", first(name), first(reference)


def paragraph_checks(ours, theirs):
    props = ours["props"]
    spacing, indent = props.get("spacing", {}), props.get("indent", {})
    mm = lambda twips: round((twips or 0) * HUNDREDTH_MM_PER_TWIP)
    first = indent.get("first_line") or 0
    if indent.get("hanging"):
        first = -indent["hanging"]
    yield "align", ADJUST.get(props.get("justification"), -1), min(theirs["adjust"], 3) if theirs["adjust"] != 4 else 2
    # "Auto" spacing replaces the stated value; both Word and LibreOffice use 14 pt for it.
    auto = lambda side: 280 if spacing.get(f"{side}_auto") else spacing.get(side)
    yield "space-before", mm(auto("before")), theirs["top"]
    yield "space-after", mm(auto("after")), theirs["bottom"]
    yield "indent-left", mm(indent.get("left")), theirs["left"]
    yield "indent-first", mm(first), theirs["first"]


def close(a, b):
    if isinstance(a, (int, float)) and not isinstance(a, bool) and isinstance(b, (int, float)):
        return abs(a - b) <= 2  # rounding between twips and 1/100 mm
    return a == b


def compare(reference_file: str):
    theirs = json.loads(Path(reference_file).read_text())
    path = theirs["path"]
    result = {"path": path, "reference_ok": theirs.get("ok", False), "reference_error": theirs.get("error")}
    run = subprocess.run([str(OCO), "inspect", "--json", path], capture_output=True)
    result["ours_ok"] = run.returncode == 0
    if not (result["ours_ok"] and result["reference_ok"]):
        return result

    ours = []
    flatten(json.loads(run.stdout)["body"], False, ours)
    their_paragraphs = theirs["paragraphs"]
    our_keys = [key("".join(r["text"] for r in p["runs"])) for p in ours]
    their_keys = [key(p["text"]) for p in their_paragraphs]
    if theirs.get("truncated"):
        our_keys = our_keys[: len(their_keys) + 50]

    their_total = sum(map(len, their_keys))
    our_total = sum(map(len, our_keys))
    # Bag-of-characters overlap: is the text there at all, regardless of how
    # it is split into paragraphs?
    overlap = sum((Counter("".join(our_keys)) & Counter("".join(their_keys))).values())
    result.update(their_chars=their_total, our_chars=our_total, shared_chars=overlap, matched_chars=0)

    stats = defaultdict(lambda: [0, 0])  # (attribute, bucket) -> [compared, different]
    examples = []
    matcher = difflib.SequenceMatcher(None, our_keys, their_keys, autojunk=False)
    for tag, i1, i2, j1, j2 in matcher.get_opcodes():
        if tag != "equal":
            continue
        for i, j in zip(range(i1, i2), range(j1, j2)):
            if not our_keys[i]:
                continue
            mine, reference = ours[i], their_paragraphs[j]
            result["matched_chars"] += len(our_keys[i])
            bucket = "table" if mine["in_table"] else "list" if mine["props"].get("numbering") else "plain"
            for attribute, a, b in paragraph_checks(mine, reference):
                entry = stats[(attribute, bucket)]
                entry[0] += 1
                if not close(a, b):
                    entry[1] += 1
                    examples.append((attribute, bucket, a, b, our_keys[i][:40]))
            a_chars, b_chars = our_chars(mine), their_chars(reference)
            if len(a_chars) != len(b_chars):
                continue  # e.g. a field we render differently; text matched only by coincidence
            seen = set()
            for (ch, a_props), (_, b_props) in zip(a_chars, b_chars):
                for attribute, a, b in char_checks(ch, a_props, b_props):
                    entry = stats[(attribute, bucket)]
                    entry[0] += 1
                    if not close(a, b):
                        entry[1] += 1
                        if (attribute, a, b) not in seen:
                            seen.add((attribute, a, b))
                            examples.append((attribute, bucket, a, b, our_keys[i][:40]))
    result["stats"] = {f"{a}|{b}": v for (a, b), v in stats.items()}
    result["examples"] = examples[:200]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("reference", help="directory written by lo_dump.py")
    parser.add_argument("--json", help="write per-document results here")
    parser.add_argument("--examples", type=int, default=4, help="documents to list per attribute")
    args = parser.parse_args()
    if not OCO.exists():
        sys.exit(f"{OCO} not found; run `cargo build --release` first")

    files = sorted(str(p) for p in Path(args.reference).glob("*.json"))
    with Pool() as pool:
        results = pool.map(compare, files, chunksize=16)

    n = len(results)
    both = [r for r in results if r["ours_ok"] and r["reference_ok"]]
    print(f"documents                     {n}")
    print(f"  opened by both              {len(both)}")
    print(f"  only by LibreOffice         {sum(1 for r in results if r['reference_ok'] and not r['ours_ok'])}")
    print(f"  only by us                  {sum(1 for r in results if r['ours_ok'] and not r['reference_ok'])}")
    print(f"  by neither                  {sum(1 for r in results if not r['ours_ok'] and not r['reference_ok'])}")
    reasons = Counter(r["reference_error"] for r in results if not r["reference_ok"])
    print(f"  LibreOffice failures        {dict(reasons)}")

    their_total = sum(r["their_chars"] for r in both)
    print(f"\ntext (characters, whitespace ignored; LibreOffice has {their_total})")
    print(f"  present in ours             {100 * sum(r['shared_chars'] for r in both) / their_total:5.1f}%")
    print(f"  in identically split paragraphs {100 * sum(r['matched_chars'] for r in both) / their_total:5.1f}%")
    exact = sum(1 for r in both if r["their_chars"] == r["matched_chars"] == r["our_chars"])
    print(f"  documents with identical text   {exact} ({100 * exact / len(both):.1f}%)")
    extra = sum(max(0, r["our_chars"] - r["shared_chars"]) for r in both)
    print(f"  characters only we produce      {extra} ({100 * extra / their_total:.1f}% of LibreOffice's)")

    totals = defaultdict(lambda: [0, 0])
    affected = defaultdict(set)
    for r in both:
        for name, (compared, different) in r["stats"].items():
            totals[name][0] += compared
            totals[name][1] += different
            if different:
                affected[name.split("|")[0]].add(r["path"])
    attributes = sorted({name.split("|")[0] for name in totals})
    print("\nformatting agreement on aligned paragraphs (share that differs)")
    print(f"  {'attribute':13s} {'plain':>9s} {'list':>9s} {'table':>9s}   documents with a difference")
    for attribute in attributes:
        cells = []
        for bucket in ("plain", "list", "table"):
            compared, different = totals.get(f"{attribute}|{bucket}", (0, 0))
            cells.append(f"{100 * different / compared:8.2f}%" if compared else "        -")
        docs = len(affected[attribute])
        print(f"  {attribute:13s} {' '.join(cells)}   {docs:5d} ({100 * docs / len(both):.1f}%)")

    print("\nmost-affected documents per attribute (ours vs LibreOffice)")
    for attribute in attributes:
        ranked = sorted(
            ((sum(v[1] for k, v in r["stats"].items() if k.split("|")[0] == attribute), r) for r in both),
            key=lambda item: -item[0],
        )
        print(f"  {attribute}")
        for count, r in ranked[: args.examples]:
            if not count:
                break
            sample = next((e for e in r["examples"] if e[0] == attribute), None)
            detail = f"{sample[2]!r} vs {sample[3]!r} in {sample[1]} text {sample[4]!r}" if sample else ""
            print(f"    {count:6d}  {r['path']}\n            {detail}")

    if args.json:
        Path(args.json).write_text(json.dumps(results, ensure_ascii=False))


if __name__ == "__main__":
    main()
