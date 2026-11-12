#!/usr/bin/env python3
"""Checks our reader against documents that Word itself re-saved.

Some public test suites (docx4j's, notably) ship pairs of files: a hand-built
probe `X.docx` that exercises an odd or invalid piece of markup, and
`X-word365.docx`, the same file opened and saved by Word 365. Word writes back
only canonical markup, so its save records how Word *understood* the probe.

If we read the probe the way Word did, then our reading of the probe and our
reading of Word's save agree. Where they differ, we interpret some markup
differently from Word. This is real Word behaviour as ground truth, obtained
without running Word.

    cargo build --release
    python3 tools/corpus/word_pairs.py corpus [--verbose]
"""

import argparse
import json
import subprocess
import sys
from collections import Counter, defaultdict
from pathlib import Path

OCO = Path(__file__).resolve().parent.parent.parent / "target/release/oco"
SUFFIX = "-word365"

# What an unspecified property means, so that "absent" and "explicitly the
# default" compare equal. Word tends to write defaults out when it saves.
DEFAULTS = {
    "bold": False, "italic": False, "caps": False, "small_caps": False, "strike": False, "vanish": False,
    "underline": "none", "color": "auto", "highlight": "none", "vert_align": "baseline",
}


def read(path: Path):
    run = subprocess.run([str(OCO), "inspect", "--json", str(path)], capture_output=True)
    if run.returncode != 0:
        return None
    paragraphs = []

    def walk(blocks):
        for block in blocks:
            if block["type"] == "paragraph":
                paragraphs.append(block)
            else:
                for row in block["rows"]:
                    for cell in row:
                        walk(cell)

    walk(json.loads(run.stdout)["body"])
    return paragraphs


def characters(paragraph):
    return [(ch, run["props"]) for run in paragraph["runs"] for ch in run["text"]]


def differences(probe, saved):
    """Yields (property, value in probe, value in Word's save) per character."""
    for (_, a), (_, b) in zip(characters(probe), characters(saved)):
        for name, default in DEFAULTS.items():
            x, y = a.get(name, default), b.get(name, default)
            if name == "color":
                x, y = str(x).upper().replace("AUTO", "auto"), str(y).upper().replace("AUTO", "auto")
            if x != y:
                yield name, x, y


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("roots", nargs="+")
    parser.add_argument("--verbose", action="store_true", help="list every differing paragraph")
    args = parser.parse_args()
    if not OCO.exists():
        sys.exit(f"{OCO} not found; run `cargo build --release` first")

    pairs = []
    for root in args.roots:
        for saved in sorted(Path(root).rglob(f"*{SUFFIX}.docx")):
            probe = saved.with_name(saved.name.replace(f"{SUFFIX}.docx", ".docx"))
            if probe.exists():
                pairs.append((probe, saved))

    outcome = Counter()
    by_group = defaultdict(Counter)
    by_property = Counter()
    details = []
    for probe, saved in pairs:
        group = probe.parent.name
        a, b = read(probe), read(saved)
        if a is None or b is None:
            # Probes of broken packages: Word repaired them, we refuse them (or vice versa).
            kind = "probe unreadable" if a is None else "save unreadable"
        else:
            a_text = ["".join(r["text"] for r in p["runs"]) for p in a]
            b_text = ["".join(r["text"] for r in p["runs"]) for p in b]
            if a_text != b_text:
                kind = "text differs"
                changed = next((i for i, (x, y) in enumerate(zip(a_text, b_text)) if x != y), min(len(a_text), len(b_text)))
                x = a_text[changed] if changed < len(a_text) else "<missing>"
                y = b_text[changed] if changed < len(b_text) else "<missing>"
                details.append(f"{probe}\n    paragraph {changed + 1}: ours {x[:70]!r}\n    {'':13s} Word {y[:70]!r}")
            else:
                found = Counter()
                for i, (p, q) in enumerate(zip(a, b)):
                    for name, x, y in set(differences(p, q)):
                        found[name] += 1
                        details.append(f"{probe}\n    paragraph {i + 1} {a_text[i][:50]!r}: {name} ours {x!r}, Word {y!r}")
                by_property.update(found.keys())
                kind = "formatting differs" if found else "agrees"
        outcome[kind] += 1
        by_group[group][kind] += 1

    print(f"{len(pairs)} probe/save pairs")
    for kind, count in outcome.most_common():
        print(f"  {count:4d}  {kind}")
    print("\nby test group")
    for group, counts in sorted(by_group.items()):
        print(f"  {group:28s} " + ", ".join(f"{c} {k}" for k, c in counts.most_common()))
    if by_property:
        print("\nproperties we read differently from Word (pairs affected)")
        for name, count in by_property.most_common():
            print(f"  {count:4d}  {name}")
    if args.verbose:
        print("\n" + "\n".join(details))


if __name__ == "__main__":
    main()
