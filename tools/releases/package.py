#!/usr/bin/env python3
"""Assemble the 0.0.1 release archives from locally verified builds.

Requires tools/web/build.sh, cargo build --release -p oco-cli, the release
demo, pinned Chinese fonts and Ubuntu's Liberation 2 / Carlito / Caladea /
DejaVu font packages. No proprietary fonts or legacy Liberation 1 are bundled.
"""
from pathlib import Path
import hashlib
import json
import shutil
import subprocess
import tarfile
import zipfile

ROOT = Path(__file__).resolve().parents[2]
VERSION = "0.0.1"
OUT = ROOT / "out/releases" / VERSION
STAGE = OUT / "stage"


def copy(source, target):
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def manifest(folder, **extra):
    files = {p.relative_to(folder).as_posix(): {"bytes": p.stat().st_size, "sha256": sha(p)}
             for p in sorted(folder.rglob("*")) if p.is_file() and p.name != "manifest.json"}
    data = {"version": VERSION, "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), **extra, "files": files}
    (folder / "manifest.json").write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n")


def common(folder):
    for name in ["LICENSE", "NOTICE", "THIRD_PARTY_NOTICES.md"]:
        copy(ROOT / name, folder / name)


def archive_zip(folder):
    output = OUT / (folder.name + ".zip")
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
        for path in sorted(folder.rglob("*")):
            if path.is_file():
                entry = zipfile.ZipInfo((Path(folder.name) / path.relative_to(folder)).as_posix(), date_time=(2026, 11, 14, 0, 0, 0))
                entry.compress_type = zipfile.ZIP_DEFLATED
                entry.external_attr = 0o100644 << 16
                archive.writestr(entry, path.read_bytes())
    return output


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    if STAGE.exists():
        shutil.rmtree(STAGE)
    web = STAGE / f"opencloudoffice-{VERSION}-web"
    sdk = STAGE / f"opencloudoffice-{VERSION}-wasm-sdk"
    cli = STAGE / f"opencloudoffice-{VERSION}-linux-x86_64"
    for folder in [web, sdk, cli]:
        common(folder)
    for name in ["index.html", "viewer.js", "viewer-client.js", "viewer-worker.js", "oco.js", "oco.wasm"]:
        copy(ROOT / "web" / name, web / name)
    for name in ["oco.js", "oco.wasm", "viewer-client.js", "viewer-worker.js"]:
        copy(ROOT / "web" / name, sdk / name)
    selected = []
    for name in ["SourceHanSerifCN-Regular.otf", "SourceHanSerifCN-Bold.otf", "SourceHanSansCN-Regular.otf", "SourceHanSansCN-Bold.otf", "LXGWWenKai-Regular.ttf"]:
        selected.append(ROOT / "fonts" / name)
    selected += sorted(Path("/usr/share/fonts/truetype/liberation2").glob("*.ttf"))
    selected += sorted(Path("/usr/share/fonts/truetype/crosextra").glob("*.ttf"))
    for family, styles in [("DejaVuSans", ["", "-Bold", "-Oblique", "-BoldOblique"]),
                           ("DejaVuSansMono", ["", "-Bold", "-Oblique", "-BoldOblique"]),
                           ("DejaVuSerif", ["", "-Bold", "-Italic", "-BoldItalic"])]:
        selected += [Path("/usr/share/fonts/truetype/dejavu") / f"{family}{style}.ttf" for style in styles]
    if len(selected) != 37:
        raise RuntimeError(f"Expected 37 release font files, found {len(selected)}; check the documented font packages")
    selected.sort(key=lambda path: path.name)
    for path in selected:
        copy(path, web / "fonts" / path.name)
    (web / "fonts/index.json").write_text(json.dumps([p.name for p in selected]) + "\n")
    font_sources = {}
    for repository, filename, dest in [("source-han-sans", "LICENSE.txt", "SourceHanSans-OFL.txt"),
                                       ("source-han-serif", "LICENSE.txt", "SourceHanSerif-OFL.txt"),
                                       ("lxgw-wenkai", "OFL.txt", "LXGWWenKai-OFL.txt")]:
        text = subprocess.check_output(["git", "-C", str(ROOT / "fonts/.src" / repository), "show", "HEAD:" + filename])
        target = web / "licenses/fonts" / dest
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(text)
        source = ROOT / "fonts/.src" / repository
        font_sources[repository] = {
            "repository": subprocess.check_output(["git", "-C", str(source), "remote", "get-url", "origin"], text=True).strip(),
            "commit": subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip(),
            "license_file": "licenses/fonts/" + dest,
        }
    for package in ["fonts-liberation2", "fonts-crosextra-carlito", "fonts-crosextra-caladea", "fonts-dejavu-core"]:
        copy(Path("/usr/share/doc") / package / "copyright", web / "licenses/fonts" / (package + ".txt"))
        font_sources[package] = {
            "ubuntu_package_version": subprocess.check_output(["dpkg-query", "-W", "-f=${Version}", package], text=True).strip(),
            "source_and_license_file": "licenses/fonts/" + package + ".txt",
        }
    copy(ROOT / "LICENSE", web / "licenses/fonts/Apache-2.0.txt")
    for path in sorted((ROOT / "testdata/zh").glob("*.docx")):
        copy(path, web / "samples" / path.name)
    demo = ROOT / "testdata/release/opencloudoffice-0.0.1.docx"
    copy(demo, web / "samples" / demo.name)
    copy(ROOT / "testdata/details/display-details.docx", web / "samples/display-details.docx")
    sample_names = [demo.name, "display-details.docx"] + [p.name for p in sorted((ROOT / "testdata/zh").glob("*.docx"))]
    (web / "samples/index.json").write_text(json.dumps(sample_names) + "\n")
    (web / "README.txt").write_text("OpencloudOffice 0.0.1 — 浏览器查看器\n\n在此目录启动静态服务器：\n  python3 -m http.server 8000 --bind 127.0.0.1\n然后打开 http://localhost:8000，选择示例文档或打开本地 DOCX。\n需要 HTTP 服务；直接双击 index.html 的 file:// 模式不能加载 Worker / WASM。\n字体和示例已包含，不需要安装 Rust。文件在浏览器本地处理。\n\n此版本为开发者预览，编辑、DOC 二进制解析、完整 Markdown 尚未实现。\n代码采用 Apache-2.0，字体许可见 licenses/fonts。\n", encoding="utf-8")
    (sdk / "README.md").write_text('''# OpencloudOffice 0.0.1 WASM SDK

Serve these files over HTTP. Import the ES module and supply your font files:

```js
import { Oco } from "./oco.js";
const core = await Oco.load("./oco.wasm");
core.addFont(await (await fetch("./fonts/your-font.ttf")).arrayBuffer());
const file = document.querySelector('input[type="file"]').files[0];
const doc = core.open(await file.arrayBuffer(), file.name);
document.querySelector("#page").innerHTML = doc.pageSvg(0);
console.log(doc.info());
doc.close();
```

This small SDK archive does not bundle fonts. The full web archive includes
open fonts and their licenses. See oco.js for the API; the viewer-client.js /
viewer-worker.js pair provides the asynchronous worker interface.
API stability is not promised for this first developer preview.
''')
    copy(ROOT / "target/release/oco", cli / "bin/oco")
    (cli / "bin/oco").chmod(0o755)
    copy(demo, cli / "samples" / demo.name)
    (cli / "README.txt").write_text("OpencloudOffice 0.0.1 — Linux x86_64 CLI\nBuilt on Ubuntu 22.04 with glibc.\n\n  ./bin/oco --version\n  ./bin/oco inspect samples/opencloudoffice-0.0.1.docx\n  ./bin/oco render samples/opencloudoffice-0.0.1.docx --fonts /path/to/web-package/fonts --out rendered\n\nThe CLI package does not include fonts. Use the fonts directory in the web package\nor provide your own .ttf / .otf / .ttc files. Render writes one SVG per page.\nThe legacy .doc reader is not implemented; Markdown is currently plain paragraphs.\nSee LICENSE and licenses/crates for licensing.\n")
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT))
    dependencies = []
    for package in metadata["packages"]:
        if package["id"] in metadata["workspace_members"]:
            continue
        source = Path(package["manifest_path"]).parent
        notices = [p for p in source.iterdir() if p.is_file() and p.name.upper().startswith(("LICENSE", "COPYING", "NOTICE"))]
        if not notices:
            raise RuntimeError("Missing license text for " + package["name"])
        for path in notices:
            copy(path, cli / "licenses/crates" / f'{package["name"]}-{package["version"]}' / path.name)
        dependencies.append({k: package[k] for k in ["name", "version", "license", "repository"]})
    manifest(web, kind="web", font_sources=font_sources, fonts="37 unmodified open font files; see licenses/fonts and THIRD_PARTY_NOTICES.md")
    manifest(sdk, kind="wasm-sdk")
    manifest(cli, kind="linux-x86_64", dependencies=dependencies)
    archives = [archive_zip(web), archive_zip(sdk)]
    native = OUT / (cli.name + ".tar.gz")
    with tarfile.open(native, "w:gz") as archive:
        archive.add(cli, arcname=cli.name)
    archives.append(native)
    (OUT / "SHA256SUMS").write_text("".join(f"{sha(p)}  {p.name}\n" for p in archives))
    for p in archives:
        print(p.name, p.stat().st_size, "bytes")


if __name__ == "__main__":
    main()
