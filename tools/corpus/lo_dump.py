#!/usr/bin/env python3
"""Dumps what LibreOffice makes of each document, as a reference to diff against.

For every paragraph in the body (including table cells) this records the text
and, per text portion, the *effective* character formatting LibreOffice
computed after its own style resolution. LibreOffice is an independent
implementation of the same file format, so wherever it and our reader agree
the result is probably right, and every disagreement is worth a look.

LibreOffice is not Word: it has import bugs of its own. Treat this as a
second opinion, not as ground truth.

    python3 tools/corpus/lo_dump.py --out corpus-lo corpus
    python3 tools/corpus/lo_dump.py --out corpus-lo --list files.txt --jobs 4

One JSON file per document is written to the output directory, named by the
SHA-1 of the document's path. Existing outputs are kept, so an interrupted
run can simply be restarted.
"""

import argparse
import hashlib
import json
import multiprocessing
import os
import shutil
import signal
import subprocess
import tempfile
import threading
import time
from pathlib import Path

import uno
from com.sun.star.beans import PropertyValue

EXTENSIONS = {".docx", ".docm", ".dotx", ".dotm"}
CHAR_PROPS = (
    "CharWeight", "CharPosture", "CharHeight", "CharFontName", "CharFontNameAsian",
    "CharUnderline", "CharStrikeout", "CharColor", "CharCaseMap", "CharHidden",
)
PARA_PROPS = (
    "ParaStyleName", "ParaAdjust", "ParaLeftMargin", "ParaRightMargin",
    "ParaFirstLineIndent", "ParaTopMargin", "ParaBottomMargin", "NumberingIsNumber", "ListId",
)
MAX_PARAGRAPHS = 600  # enough to compare formatting; keeps huge documents cheap


def out_name(path: str) -> str:
    return hashlib.sha1(path.encode()).hexdigest() + ".json"


class Office:
    """One headless soffice process with its own profile, restarted on demand."""

    def __init__(self, index: int):
        self.port = 2700 + index
        self.profile = None
        self.process = None
        self.desktop = None

    def start(self) -> None:
        self.stop()
        # A fresh profile each time: one left behind by a crash can make the
        # next start hang on a lock file or a recovery prompt.
        self.profile = tempfile.mkdtemp(prefix=f"lo-profile-{self.port}-")
        self.process = subprocess.Popen(
            [
                "soffice", "--headless", "--invisible", "--norestore", "--nologo", "--nodefault",
                f"-env:UserInstallation=file://{self.profile}",
                f"--accept=socket,host=127.0.0.1,port={self.port};urp;",
            ],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            # `soffice` is a wrapper script; its own group lets us kill the
            # real soffice.bin along with it.
            start_new_session=True,
        )
        resolver = uno.getComponentContext().ServiceManager.createInstance("com.sun.star.bridge.UnoUrlResolver")
        url = f"uno:socket,host=127.0.0.1,port={self.port};urp;StarOffice.ComponentContext"
        for _ in range(120):
            try:
                context = resolver.resolve(url)
                self.desktop = context.ServiceManager.createInstanceWithContext("com.sun.star.frame.Desktop", context)
                return
            except Exception:
                time.sleep(0.25)
        raise RuntimeError("soffice did not start")

    def stop(self) -> None:
        process, self.process, self.desktop = self.process, None, None
        if process is not None:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
        if self.profile is not None:
            shutil.rmtree(self.profile, ignore_errors=True)
            self.profile = None

    def close(self) -> None:
        self.stop()


def prop(name: str, value) -> PropertyValue:
    p = PropertyValue()
    p.Name, p.Value = name, value
    return p


def dump_paragraph(paragraph, in_table: bool) -> dict:
    values = paragraph.getPropertyValues(PARA_PROPS)
    record = {
        "text": paragraph.getString(),
        "in_table": in_table,
        "style": values[0],
        "adjust": int(values[1]),
        "left": values[2], "right": values[3], "first": values[4],
        "top": values[5], "bottom": values[6],
        "list": bool(values[7]) and bool(values[8]),
        "portions": [],
    }
    for portion in paragraph:
        kind = portion.TextPortionType
        if kind not in ("Text", "TextField"):
            continue
        text = portion.getString()
        if not text:
            continue
        v = portion.getPropertyValues(CHAR_PROPS)
        record["portions"].append({
            "t": text, "field": kind == "TextField",
            "weight": v[0], "italic": v[1].value != "NONE", "size": v[2],
            "font": v[3], "font_asian": v[4],
            "underline": int(v[5]), "strike": int(v[6]), "color": int(v[7]),
            "case": int(v[8]), "hidden": bool(v[9]),
        })
    return record


def dump_text(text, in_table: bool, out: list) -> None:
    for element in text:
        if len(out) >= MAX_PARAGRAPHS:
            return
        if element.supportsService("com.sun.star.text.Paragraph"):
            out.append(dump_paragraph(element, in_table))
        elif element.supportsService("com.sun.star.text.TextTable"):
            for name in element.getCellNames():
                dump_text(element.getCellByName(name), True, out)


def dump_document(office: Office, path: str) -> dict:
    started = time.perf_counter()
    document = office.desktop.loadComponentFromURL(
        uno.systemPathToFileUrl(os.path.abspath(path)), "_blank", 0,
        (prop("Hidden", True), prop("ReadOnly", True), prop("UpdateDocMode", 0)),
    )
    if document is None:
        return {"ok": False, "error": "load-failed"}
    try:
        record = {"ok": True, "load_ms": round((time.perf_counter() - started) * 1000)}
        if not document.supportsService("com.sun.star.text.TextDocument"):
            return {"ok": False, "error": "not-a-text-document"}
        paragraphs = []
        dump_text(document.Text, False, paragraphs)
        record["paragraphs"] = paragraphs
        record["truncated"] = len(paragraphs) >= MAX_PARAGRAPHS
        try:
            record["pages"] = int(document.CurrentController.PageCount)
        except Exception:
            record["pages"] = None
        return record
    finally:
        document.close(True)


def worker(index: int, queue, out_dir: str, timeout: float) -> None:
    office = Office(index)
    office.start()
    while True:
        path = queue.get()
        if path is None:
            break
        target = Path(out_dir) / out_name(path)
        # A document that hangs or crashes soffice must not stall the run.
        watchdog = threading.Timer(timeout, office.stop)
        watchdog.start()
        failed = False
        try:
            record = dump_document(office, path)
        except Exception as error:  # includes the bridge dying under the watchdog
            failed = True
            hung = office.process is None
            record = {"ok": False, "error": "timeout" if hung else "crash", "detail": type(error).__name__}
        finally:
            watchdog.cancel()
        record["path"] = path
        target.write_text(json.dumps(record, ensure_ascii=False))
        if failed:
            office.start()
    office.close()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("roots", nargs="*", help="directories to scan for Word files")
    parser.add_argument("--list", help="file with one document path per line, instead of scanning")
    parser.add_argument("--out", required=True, help="output directory")
    parser.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    parser.add_argument("--timeout", type=float, default=45.0, help="seconds allowed per document")
    args = parser.parse_args()

    if args.list:
        paths = [line.strip() for line in Path(args.list).read_text().splitlines() if line.strip()]
    else:
        paths = sorted(
            str(p) for root in args.roots for p in Path(root).rglob("*")
            if p.suffix.lower() in EXTENSIONS and ".git" not in p.parts and p.is_file()
        )
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    todo = [p for p in paths if not (out_dir / out_name(p)).exists()]
    print(f"{len(paths)} documents, {len(todo)} to do", flush=True)

    queue = multiprocessing.Queue()
    for path in todo:
        queue.put(path)
    jobs = max(1, min(args.jobs, len(todo)))
    for _ in range(jobs):
        queue.put(None)
    workers = [multiprocessing.Process(target=worker, args=(i, queue, str(out_dir), args.timeout)) for i in range(jobs)]
    for w in workers:
        w.start()
    for w in workers:
        w.join()
    print("done", flush=True)


if __name__ == "__main__":
    main()
