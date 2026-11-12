#!/usr/bin/env python3
"""Has WPS Office lay out a folder of documents, as a second reference for
tools/word/compare.py: one PDF per document, in the folder's `wps` subfolder.

    python3 tools/wps/export.py [testdata/zh]

Needs, on Linux:

  - WPS Office for Linux installed in /opt/kingsoft/wps-office;
  - its licence agreement accepted by you. Start WPS once and accept it, or
    add `common\\AcceptedEULA=true` under `[6.0]` in
    ~/.config/Kingsoft/Office.conf. This script does not accept it for you.
    Note that the free edition is licensed for personal, non-commercial use;
  - `pip install pywpsrpc`, the bindings to WPS's automation interface;
  - an X display. Without one the script runs itself under xvfb-run.

WPS finds fonts through fontconfig. A font the documents name but the
machine lacks is replaced by WPS with one of its choosing, and the layout
then follows that font's measurements, not the named font's. For a
reference worth having, run this where the fonts are installed, or on
documents that name fonts you have.

NOTE: not run yet. It was written against pywpsrpc's documented interface;
WPS itself could not be started here because its licence had not been
accepted by the person it binds.
"""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
CONFIG = Path.home() / ".config/Kingsoft/Office.conf"


def accepted_licence():
    try:
        return any(line.strip().lower() == "common\\acceptedeula=true" for line in CONFIG.read_text().splitlines())
    except OSError:
        return False


def export(documents: Path, out: Path):
    from pywpsrpc.common import S_OK, QtApp
    from pywpsrpc.rpcwpsapi import createWpsRpcInstance, wpsapi

    files = sorted(
        p for p in documents.rglob("*")
        if p.suffix.lower() in (".docx", ".docm") and out not in p.parents and not p.name.startswith("~$")
    )
    if not files:
        sys.exit(f"no .docx files under {documents}")

    _qt = QtApp(sys.argv)  # the bindings need a Qt application object to exist
    hr, rpc = createWpsRpcInstance()
    if hr != S_OK:
        sys.exit(f"could not reach WPS (createWpsRpcInstance returned {hr:#x})")
    hr, app = rpc.getWpsApplication()
    if hr != S_OK:
        sys.exit(f"could not start WPS (getWpsApplication returned {hr:#x})")
    app.Visible = False

    exported = {}
    try:
        for file in files:
            relative = file.relative_to(documents)
            pdf = (out / relative).with_suffix(".pdf")
            pdf.parent.mkdir(parents=True, exist_ok=True)
            hr, doc = app.Documents.Open(str(file), ReadOnly=True)
            if hr != S_OK:
                print(f"{relative}: WPS could not open it ({hr:#x})")
                continue
            try:
                hr = doc.SaveAs2(str(pdf), FileFormat=wpsapi.wdFormatPDF)
                if hr == S_OK:
                    exported[relative.as_posix()] = {}
                    print(relative)
                else:
                    print(f"{relative}: WPS could not export it ({hr:#x})")
            finally:
                doc.Close(wpsapi.wdDoNotSaveChanges)
        manifest = {"application": "WPS Office for Linux", "documents": exported}
        try:
            manifest["version"] = f"{app.Version} build {app.Build}"
        except Exception:  # not every build answers these
            pass
        (out / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=1), encoding="utf-8")
        print(f"{len(exported)} documents -> {out}")
    finally:
        app.Quit(wpsapi.wdDoNotSaveChanges)


def main():
    documents = (ROOT / (sys.argv[1] if len(sys.argv) > 1 else "testdata/zh")).resolve()
    if not accepted_licence():
        sys.exit(
            "WPS Office's licence agreement has not been accepted on this machine.\n"
            "Start WPS once and accept it, or add `common\\AcceptedEULA=true` under `[6.0]`\n"
            f"in {CONFIG}. Read it first: the free edition is for personal, non-commercial use."
        )
    if not os.environ.get("DISPLAY"):
        if not shutil.which("xvfb-run"):
            sys.exit("no X display, and xvfb-run is not installed")
        sys.exit(subprocess.run(["xvfb-run", "-a", sys.executable, *sys.argv]).returncode)
    export(documents, documents / "wps")
    # WPS's own threads can crash the interpreter on the way out; the work is done.
    sys.stdout.flush()
    os._exit(0)


if __name__ == "__main__":
    main()
