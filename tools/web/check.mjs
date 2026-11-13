// Checks that the WebAssembly build draws every page exactly as the native
// command-line tool does: same pages, same bytes of SVG.
//
//   tools/web/build.sh && cargo build --release && node tools/web/check.mjs [file.docx ...]
//
// With no arguments it checks the documents in testdata/zh.

import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { Oco } from "../../web/oco.js";

const root = resolve(fileURLToPath(import.meta.url), "../../..");
const web = join(root, "web");
const oco = await Oco.load(readFileSync(join(web, "oco.wasm")));
let faces = 0;
for (const font of JSON.parse(readFileSync(join(web, "fonts/index.json"), "utf8"))) {
  faces += oco.addFont(readFileSync(join(web, "fonts", font)));
}

let files = process.argv.slice(2);
if (files.length === 0) {
  const samples = join(root, "testdata/zh");
  files = readdirSync(samples).filter((name) => name.endsWith(".docx")).sort().map((name) => join(samples, name));
}

let pages = 0;
let different = 0;
let checked = 0;
let failed = 0;
let wasmTime = 0;
for (const file of files) {
  const out = mkdtempSync(join(tmpdir(), "oco-check-"));
  try {
    execFileSync(join(root, "target/release/oco"), ["render", file, "--out", out], { cwd: root, stdio: "ignore" });
    const began = performance.now();
    const doc = oco.open(readFileSync(file), file);
    let drawn;
    let count;
    try {
      count = doc.pageCount;
      drawn = Array.from({ length: count }, (_, page) => doc.pageSvg(page));
    } finally { doc.close(); }
    wasmTime += performance.now() - began;

    const native = readdirSync(out).filter((name) => name.endsWith(".svg")).length;
    let same = native === count;
    for (let page = 0; same && page < count; page++) {
      same = readFileSync(join(out, `page-${page + 1}.svg`), "utf8") === drawn[page];
    }
    checked++;
    pages += count;
    if (!same) {
      different++;
      console.log(`DIFFERENT ${file} (native ${native} pages, wasm ${count})`);
    }
  } catch (error) {
    failed++;
    console.error(`FAILED ${file}: ${error.message}`);
  } finally {
    rmSync(out, { recursive: true });
  }
}
console.log(`${faces} font faces; ${checked}/${files.length} documents checked, ${pages} pages; ${different} differ, ${failed} failed; wasm ${Math.round(wasmTime)} ms`);
process.exit(different || failed || !checked ? 1 : 0);
