#!/usr/bin/env python3
"""Has WPS Office lay out a folder of documents, as a second reference for
tools/word/compare.py: one PDF per document, in the folder's `wps` subfolder.

    python3 tools/wps/export.py [testdata/zh-open]

Needs, on Linux:

  - WPS Office for Linux installed in /opt/kingsoft/wps-office;
  - its licence agreement accepted by you. Start WPS once and accept it, or
    add `common\\AcceptedEULA=true` under `[6.0]` in
    ~/.config/Kingsoft/Office.conf. This script does not accept it for you.
    Note that the free edition is licensed for personal, non-commercial use;
  - `pip install pywpsrpc pdfplumber`: the bindings to WPS's automation
    interface (they load the system's Qt 5 core library), and a PDF reader;
  - Xvfb, when there is no X display.

What it took to make WPS 11.1.0.11723 run on Ubuntu 24.04, for the record:
the packages libqt5core5t64, libqt5xml5t64 and libxkbcommon-x11-0; WPS's own
outdated libstdc++.so.6 moved out of office6/, so that the system's is used;
and a link office6/libtiff.so.5 to the system's libtiff.so.6, without which
PDF export fails with 0x80010105.

Fonts. WPS finds fonts through fontconfig. A font the documents name but the
machine lacks is replaced by WPS with one of its choosing, and the layout
then follows that font's measurements, not the named font's. So compare on
documents that name fonts the machine has: testdata/zh-open names the open
fonts in ./fonts, which this script makes visible to WPS through
tools/fonts/fontconfig.conf. The manifest it writes lists the fonts each PDF
really uses.

WPS embeds most of each font, so its PDFs run to megabytes and are not
committed (.gitignore). Beside each PDF the script writes the lines read
from it, `<name>.lines.json`, which is what the comparison needs and is
committed in the PDF's place.

The script also sets two of WPS's preferences, which only silence prompts
that would block an unattended start: the warning about missing symbol
fonts, and the question whether to make WPS the default office application
(it is not made the default).
"""

import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
sys.path.insert(0, str(ROOT / "tools"))
from common.layout_lines import write_lines_file  # noqa: E402

CONFIG = Path.home() / ".config/Kingsoft/Office.conf"
WPS = "/opt/kingsoft/wps-office/office6/wps"
QUIET = ["common\\system_check\\no_necessary_symbol_fonts", "common\\do_not_detect_file_association_while_startup"]


def accepted_licence():
    try:
        return any(line.strip().lower() == "common\\acceptedeula=true" for line in CONFIG.read_text().splitlines())
    except OSError:
        return False


def silence_prompts():
    """Sets the preferences in QUIET to true, under the `[6.0]` section."""
    lines = [l for l in CONFIG.read_text().splitlines() if l.split("=")[0] not in QUIET]
    section = lines.index("[6.0]") + 1
    lines[section:section] = [f"{key}=true" for key in QUIET]
    CONFIG.write_text("\n".join(lines) + "\n")


def display():
    """An X display for WPS: the one there is, or a virtual one started here.

    xvfb-run will not do: WPS is started by the automation library without
    the X authority file xvfb-run sets up, and is refused by the server. The
    virtual display here has access control off and listens locally only.
    """
    if os.environ.get("DISPLAY"):
        return None
    read, write = os.pipe()
    server = subprocess.Popen(
        ["Xvfb", "-displayfd", str(write), "-screen", "0", "1280x1024x24", "-ac", "-nolisten", "tcp"],
        pass_fds=[write], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    os.close(write)
    os.environ["DISPLAY"] = ":" + os.read(read, 16).decode().strip()
    return server


def fonts_of(pdf: Path):
    """Names of the fonts a PDF uses, without their subset prefixes."""
    import pdfplumber

    with pdfplumber.open(pdf) as document:
        names = {char["fontname"] for page in document.pages for char in page.chars}
    return sorted({re.sub(r"^[A-Z]{6}\+", "", name) for name in names})


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
        sys.exit(f"could not load WPS's automation library ({hr & 0xFFFFFFFF:#x})")
    # Not setStartTimeout: with it the library gives up before WPS is up.
    rpc.setProcessPath(WPS)
    hr, app = rpc.getWpsApplication()
    if hr != S_OK:
        sys.exit(f"could not start WPS ({hr & 0xFFFFFFFF:#x}); see this script's notes on what it needs")
    app.Visible = False

    exported = {}
    try:
        for file in files:
            relative = file.relative_to(documents)
            pdf = (out / relative).with_suffix(".pdf")
            pdf.parent.mkdir(parents=True, exist_ok=True)
            hr, doc = app.Documents.Open(str(file), ReadOnly=True)
            if hr != S_OK:
                print(f"{relative}: WPS could not open it ({hr & 0xFFFFFFFF:#x})")
                continue
            try:
                _, pages = doc.ComputeStatistics(wpsapi.wdStatisticPages)
                hr = doc.SaveAs2(str(pdf), FileFormat=wpsapi.wdFormatPDF)
                if hr == S_OK:
                    write_lines_file(pdf)
                    exported[relative.as_posix()] = {"pages": pages, "fonts": fonts_of(pdf)}
                    print(f"{relative}: {pages} pages; {', '.join(exported[relative.as_posix()]['fonts'])}")
                else:
                    print(f"{relative}: WPS could not export it ({hr & 0xFFFFFFFF:#x})")
            finally:
                doc.Close(wpsapi.wdDoNotSaveChanges)
        manifest = {
            "application": f"WPS Office for Linux {app.Build}",
            "export": "SaveAs2 with wdFormatPDF",
            "documents": exported,
        }
        (out / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
        print(f"{len(exported)} documents -> {out}")
    finally:
        app.Quit(wpsapi.wdDoNotSaveChanges)


def main():
    documents = (ROOT / (sys.argv[1] if len(sys.argv) > 1 else "testdata/zh-open")).resolve()
    if not accepted_licence():
        sys.exit(
            "WPS Office's licence agreement has not been accepted on this machine.\n"
            "Start WPS once and accept it, or add `common\\AcceptedEULA=true` under `[6.0]`\n"
            f"in {CONFIG}. Read it first: the free edition is for personal, non-commercial use."
        )
    silence_prompts()
    os.environ.setdefault("FONTCONFIG_FILE", str(ROOT / "tools/fonts/fontconfig.conf"))
    server = display()
    status = 0
    try:
        export(documents, documents / "wps")
    except SystemExit as stop:
        print(stop.code, file=sys.stderr)
        status = 1
    finally:
        sys.stdout.flush()
        sys.stderr.flush()
        if server:
            server.terminate()
        # Leave at once: WPS's own threads can crash the interpreter on its way out.
        os._exit(status)


if __name__ == "__main__":
    main()
