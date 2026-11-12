#!/usr/bin/env python3
"""The fidelity scoreboard: how closely we follow Word, as numbers that may only go up.

Everything here is measured against Word itself, with no copy of Word in the loop:

  reading   Word re-saved a probe document. Do we read the probe as Word did?
            (tools/corpus/word_pairs.py)
  layout    Word recorded in each file how many pages it had and where each
            page began. Do our pages begin in the same places?
            (oco score)
  lines     Word exported a PDF of a document. Do we break every line and
            every page where Word did? Counted only for the document sets
            that have Word's PDFs beside them (tools/word/compare.py).

The result is compared with the baseline kept beside this script. A document
that agreed with Word when the baseline was cut must still agree: that is the
ratchet. Agreement can only be added.

    cargo build --release
    tools/fidelity/score.py            # measure and compare with the baseline
    tools/fidelity/score.py --update   # ... and record the result as the new baseline

Exit status: 0 when nothing got worse; 1 when a document that agreed with Word
no longer does; 2 when the measurement cannot be compared with the baseline,
because the corpus or the fonts are not the ones the baseline was cut with.
"""

import argparse
import hashlib
import json
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
OCO = ROOT / "target/release/oco"
BASELINE = Path(__file__).resolve().parent / "baseline.json"

sys.path.insert(0, str(ROOT / "tools"))
from word.compare import measure as measure_against_pdfs  # noqa: E402

# Document sets that may have Word's own PDFs in a `word` subfolder.
WORD_SETS = ["testdata/zh"]

# The groups of documents `oco score` reports, widest first.
GROUPS = [
    ("all", "Word 保存过的全部文档"),
    ("fresh", "……且内容自 Word 保存后未被改动"),
    ("plain", "……且只含正文（无表格、图片、分栏等）"),
    ("covered", "……且所用字体齐全"),
]


def run(*command):
    done = subprocess.run([str(c) for c in command], cwd=ROOT, capture_output=True, text=True)
    if done.returncode != 0:
        sys.exit(f"{' '.join(str(c) for c in command)} failed:\n{done.stdout}{done.stderr}")
    return done.stdout


def environment(corpus: Path, font_dirs):
    """What the figures depend on besides our own code."""
    commits = {}
    for source in sorted(p for p in corpus.iterdir() if (p / ".git").exists()):
        commits[source.name] = run("git", "-C", source, "rev-parse", "HEAD").strip()
    fonts = {}
    for directory in font_dirs:
        directory = (ROOT / directory) if not Path(directory).is_absolute() else Path(directory)
        for file in sorted(directory.rglob("*")) if directory.is_dir() else []:
            hidden = any(part.startswith(".") for part in file.relative_to(directory).parts)
            if file.is_file() and not hidden and file.suffix.lower() in (".ttf", ".otf", ".ttc"):
                fonts[file.name] = hashlib.sha256(file.read_bytes()).hexdigest()[:16]
    return {"corpus": commits, "fonts": fonts}


def measure(corpus: Path):
    with tempfile.TemporaryDirectory() as scratch:
        layout_file, pairs_file = Path(scratch) / "layout.json", Path(scratch) / "pairs.json"
        run(OCO, "score", corpus, "--json", layout_file)
        run(sys.executable, ROOT / "tools/corpus/word_pairs.py", corpus, "--json", pairs_file)
        layout = json.loads(layout_file.read_text(encoding="utf-8"))
        pairs = json.loads(pairs_file.read_text(encoding="utf-8"))["pairs"]

    def relative(path):
        return Path(path).resolve().relative_to(corpus.resolve()).as_posix()

    documents = {}
    for doc in layout["documents"]:
        starts = doc["page_starts"] + doc["forced_page_starts"]
        # Only where Word's record still describes the content, and has
        # something to say about pagination.
        if doc["fresh"] and (starts or doc["word_pages"] >= 2):
            documents[relative(doc["path"])] = {
                "pages": [doc["word_pages"], doc["pages"]],
                "starts": [starts, doc["page_starts_equal"] + doc["forced_page_starts_equal"]],
            }
    reading = {relative(path): outcome for path, outcome in pairs.items()}

    lines = {}
    for name in WORD_SETS:
        results = measure_against_pdfs(ROOT / name, ROOT / name / "word", ROOT / "out/word-compare" / Path(name).name)
        lines.update({f"{name}/{document}": result for document, result in results.items()})
    total = lambda field: sum(result[field] for result in lines.values())
    return {
        "environment": environment(corpus, layout["font_dirs"]),
        "reading": {"pairs": len(reading), "agree": sum(o == "agrees" for o in reading.values())},
        "layout": {name: layout["groups"][name] for name, _ in GROUPS},
        "lines": {
            "documents": len(lines),
            "pages_equal": sum(r["pages"][0] == r["pages"][1] for r in lines.values()),
            "lines": total("lines"),
            "same": total("same"),
            "page_starts": total("page_starts"),
            "page_starts_equal": total("page_starts_equal"),
        },
        "documents": {"reading": reading, "layout": documents, "lines": lines},
    }


def share(count, total):
    return f"{count}/{total}" + (f" {100 * count / total:5.1f}%" if total else "")


def delta(now, before):
    """The change since the baseline, in a column of its own width."""
    return f"{'' if before is None or now == before else f'({now - before:+d})':>7}"


def width(text):
    """Columns the text takes in a terminal: Chinese characters take two."""
    return sum(2 if ord(ch) > 0x2E7F else 1 for ch in text)


def right(text, columns):
    return " " * max(0, columns - width(text)) + text


def scoreboard(current, baseline):
    """The table, with each figure's change since the baseline."""
    old = baseline or {}
    reading = current["reading"]
    lines = [
        "读取：与 Word 重存结果一致的探针文档",
        f"  {share(reading['agree'], reading['pairs'])}" + delta(reading["agree"], old.get("reading", {}).get("agree")),
        "",
        "排版：对照 Word 记在文件里的分页结果",
        f"  {'':38}{right('文档', 6)}"
        + "".join(right(title, 17) + " " * 7 for title in ("页数一致(≥2页)", "页首位置一致", "页首在同一段")),
    ]
    for name, title in GROUPS:
        group, before = current["layout"][name], old.get("layout", {}).get(name, {})
        cells = [
            (group["multipage_pages_equal"], group["multipage"], "multipage_pages_equal"),
            (group["page_starts_equal"], group["page_starts"], "page_starts_equal"),
            (group["page_starts_same_paragraph"], group["page_starts"], "page_starts_same_paragraph"),
        ]
        lines.append(
            f"  {title}{' ' * (38 - width(title))}{group['documents']:>6}"
            + "".join(f"{share(count, total):>17}" + delta(count, before.get(key)) for count, total, key in cells)
        )
    forced = current["layout"]["fresh"]
    lines.append(
        f"  （另有分页符等强制出来的页首 {forced['forced_page_starts']} 处，"
        f"还原 {forced['forced_page_starts_equal']} 处，不计入上表）"
    )
    lines += ["", "逐行：对照 Word 导出的 PDF"]
    exact, before = current["lines"], old.get("lines", {})
    if exact["documents"]:
        lines.append(
            f"  {exact['documents']} 份文档；页数一致 {exact['pages_equal']}"
            + delta(exact["pages_equal"], before.get("pages_equal")).rstrip()
            + f"；断行一致 {share(exact['same'], exact['lines'])}"
            + delta(exact["same"], before.get("same")).rstrip()
            + f"；页首一致 {share(exact['page_starts_equal'], exact['page_starts'])}"
            + delta(exact["page_starts_equal"], before.get("page_starts_equal")).rstrip()
        )
    else:
        lines.append("  还没有 Word 导出的 PDF。用 tools/word/export.ps1 在装有 Word 的 Windows 上生成（见 docs/fidelity.md）。")
    return "\n".join(line.rstrip() for line in lines)


def compare(current, baseline):
    """Documents that got worse, got better, and joined or left the measured set."""
    worse, better, gone, new = [], [], [], []
    for path, before in baseline["documents"]["reading"].items():
        now = current["documents"]["reading"].get(path)
        if now is None:
            gone.append(f"读取 {path}")
        elif before == "agrees" and now != "agrees":
            worse.append(f"读取 {path}: 原先与 Word 一致，现在 {now}")
        elif before != "agrees" and now == "agrees":
            better.append(f"读取 {path}: 现在与 Word 一致")
    for path, before in baseline["documents"]["layout"].items():
        now = current["documents"]["layout"].get(path)
        if now is None:
            gone.append(f"排版 {path}")
            continue
        changes = []
        pages_before, pages_now = before["pages"][0] == before["pages"][1], now["pages"][0] == now["pages"][1]
        if pages_before != pages_now:
            changes.append((pages_now, f"页数 Word {now['pages'][0]}，我们 {before['pages'][1]} → {now['pages'][1]}"))
        if before["starts"] != now["starts"]:
            gained = now["starts"][1] - before["starts"][1]
            if gained or before["starts"][0] != now["starts"][0]:
                changes.append(
                    (gained >= 0, f"页首还原 {before['starts'][1]}/{before['starts'][0]} → {now['starts'][1]}/{now['starts'][0]}")
                )
        for improved, text in changes:
            (better if improved else worse).append(f"排版 {path}: {text}")
    for path, before in baseline["documents"].get("lines", {}).items():
        now = current["documents"]["lines"].get(path)
        if now is None:
            gone.append(f"逐行 {path}")
            continue
        for field, title in (("same", "断行一致"), ("page_starts_equal", "页首一致")):
            if now[field] != before[field]:
                (better if now[field] > before[field] else worse).append(
                    f"逐行 {path}: {title} {before[field]} → {now[field]}"
                )
        pages_before, pages_now = before["pages"][0] == before["pages"][1], now["pages"][0] == now["pages"][1]
        if pages_before != pages_now:
            (better if pages_now else worse).append(
                f"逐行 {path}: 页数 Word {now['pages'][0]}，我们 {before['pages'][1]} → {now['pages'][1]}"
            )
    for kind, name in (("reading", "读取"), ("layout", "排版"), ("lines", "逐行")):
        known = baseline["documents"].get(kind, {})
        new += [f"{name} {p}" for p in current["documents"][kind] if p not in known]
    return worse, better, gone, new


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--corpus", default="corpus", help="the corpus directory (default: corpus)")
    parser.add_argument("--update", action="store_true", help="record the result as the new baseline")
    parser.add_argument(
        "--force", action="store_true", help="with --update: record it even though some documents got worse"
    )
    args = parser.parse_args()
    corpus = (ROOT / args.corpus).resolve()
    if not OCO.exists():
        sys.exit(f"{OCO} not found; run `cargo build --release` first")
    if not corpus.is_dir():
        sys.exit(f"{corpus} not found; run tools/corpus/fetch.sh first")

    current = measure(corpus)
    baseline = json.loads(BASELINE.read_text(encoding="utf-8")) if BASELINE.exists() else None

    comparable = True
    if baseline:
        for what, name in (("corpus", "测试集"), ("fonts", "字体")):
            before, now = baseline["environment"][what], current["environment"][what]
            if before != now:
                comparable = False
                print(f"{name}与基线不同，数字不可比：")
                for key in sorted(set(before) | set(now)):
                    if before.get(key) != now.get(key):
                        print(f"  {key}: 基线 {before.get(key, '无')}，现在 {now.get(key, '无')}")
                print()

    print(scoreboard(current, baseline if comparable else None))
    status = 0
    worse = []
    if baseline and comparable:
        worse, better, gone, new = compare(current, baseline)
        for title, items in (("变差", worse), ("变好", better), ("不再参与度量", gone), ("新加入度量", new)):
            if items:
                print(f"\n{title}（{len(items)}）")
                print("\n".join(f"  {item}" for item in items[:40]))
                if len(items) > 40:
                    print(f"  …… 另有 {len(items) - 40} 条")
        if worse:
            print("\n有文档的还原度下降了。基线只许升不许降：请修复，或确认后用 --update --force 记录。")
            status = 1
        elif not (better or gone or new):
            print("\n与基线一致。")
    elif baseline:
        print("\n未与基线比较。换回基线所用的测试集和字体，或用 --update 以当前环境重新建立基线。")
        status = 2
    else:
        print("\n还没有基线。用 --update 建立。")

    if args.update:
        if worse and not args.force:
            sys.exit("没有更新基线：有文档变差了（确认无误后加 --force）。")
        BASELINE.write_text(json.dumps(current, ensure_ascii=False, indent=1, sort_keys=True) + "\n", encoding="utf-8")
        print(f"\n基线已写入 {BASELINE.relative_to(ROOT)}")
        status = 0
    sys.exit(status)


if __name__ == "__main__":
    main()
