// Verify staged release archives' contents and native/WASM parity using the
// release's own fonts. Run tools/releases/package.py first.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readFile, readdir, mkdir } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL, fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(import.meta.url), "../../..");
const stage = resolve(root, "out/releases/0.0.1/stage");
const web = resolve(stage, "opencloudoffice-0.0.1-web");
const sdk = resolve(stage, "opencloudoffice-0.0.1-wasm-sdk");
const cli = resolve(stage, "opencloudoffice-0.0.1-linux-x86_64");
let files = 0;
for (const folder of [web, sdk, cli]) {
  const manifest = JSON.parse(await readFile(resolve(folder, "manifest.json"), "utf8"));
  assert.equal(manifest.version, "0.0.1");
  for (const [path, expected] of Object.entries(manifest.files)) {
    const data = await readFile(resolve(folder, path));
    assert.equal(data.length, expected.bytes, path);
    assert.equal(createHash("sha256").update(data).digest("hex"), expected.sha256, path);
    files++;
  }
}
assert.equal(execFileSync(resolve(cli, "bin/oco"), ["--version"], { encoding: "utf8" }).trim(), "OpencloudOffice 0.0.1");
assert.deepEqual(await readFile(resolve(web, "oco.wasm")), await readFile(resolve(sdk, "oco.wasm")));
const { Oco } = await import(pathToFileURL(resolve(sdk, "oco.js")));
const core = await Oco.load(await readFile(resolve(sdk, "oco.wasm")));
const fonts = JSON.parse(await readFile(resolve(web, "fonts/index.json"), "utf8"));
assert.equal(fonts.length, 37);
for (const name of fonts) core.addFont(await readFile(resolve(web, "fonts", name)));
const documents = [
  resolve(web, "samples/opencloudoffice-0.0.1.docx"),
  resolve(web, "samples/display-details.docx"),
  resolve(root, "testdata/probes/notes-each-page.docx"),
];
let pages = 0;
for (const [index, file] of documents.entries()) {
  const out = resolve(root, "out/releases/0.0.1/checks", String(index));
  await mkdir(out, { recursive: true });
  execFileSync(resolve(cli, "bin/oco"), ["render", file, "--fonts", resolve(web, "fonts"), "--out", out], { stdio: "pipe" });
  const doc = core.open(await readFile(file), file);
  try {
    const native = (await readdir(out)).filter((name) => name.endsWith(".svg"));
    assert.equal(doc.pageCount, native.length, file);
    if (index === 0) assert.equal(doc.pageCount, 2);
    for (let page = 0; page < doc.pageCount; page++) {
      assert.equal(doc.pageSvg(page), await readFile(resolve(out, `page-${page + 1}.svg`), "utf8"), `${file}: page ${page + 1}`);
      pages++;
    }
  } finally { doc.close(); }
}
console.log(`PASS ${files} manifest entries; CLI version; 37 fonts; ${documents.length} documents / ${pages} pages match between release CLI and SDK`);
