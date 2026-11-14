// Font parsing, layout, search and SVG generation all run off the UI thread.
import { Oco } from "./oco.js";

let core;
const documents = new Map();
let revision = 0;
let fonts = 0;
let initialized;

async function initialize() {
  core = await Oco.load(new URL("oco.wasm", import.meta.url));
  let files = [];
  const failed = [];
  try {
    const response = await fetch(new URL("fonts/index.json", import.meta.url));
    if (!response.ok) throw new Error("字体清单不可用");
    files = await response.json();
    if (!Array.isArray(files) || files.some((file) => typeof file !== "string")) throw new Error("字体清单格式不正确");
  } catch {
    files = [];
    failed.push("字体清单");
  }
  let done = 0;
  let bytes = 0;
  // Keep at most four downloaded files outside WASM at once. Font insertion
  // order is the manifest's, regardless of which download finishes first.
  for (let start = 0; start < files.length; start += 4) {
    const batch = await Promise.all(files.slice(start, start + 4).map(async (file) => {
      try {
        const response = await fetch(new URL(`fonts/${encodeURIComponent(file)}`, import.meta.url));
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        const data = await response.arrayBuffer();
        bytes += data.byteLength;
        return data;
      } catch {
        failed.push(file);
        return null;
      } finally {
        done++;
        self.postMessage({ type: "progress", done, total: files.length, bytes });
      }
    }));
    for (let index = 0; index < batch.length; index++) {
      const data = batch[index];
      if (data) {
        const added = core.addFont(data);
        fonts += added;
        if (!added) failed.push(files[start + index]);
      }
      batch[index] = null;
    }
  }
  return { fonts, failed };
}

function summary(doc, began) {
  const count = doc.pageCount;
  const sizes = Array.from({ length: count }, (_, page) => doc.pageSize(page));
  return { sizes, info: doc.info(), elapsed: performance.now() - began };
}

async function handle(type, args) {
  if (type === "init") return initialized ??= initialize();
  await initialized;
  if (!core) throw new Error("排版内核尚未加载。");
  switch (type) {
    case "open": {
      if (!fonts) throw new Error("没有可用的字体，请先点击“添加字体”。");
      const began = performance.now();
      const next = core.open(args.bytes, args.name);
      let result;
      try {
        result = summary(next, began);
      } catch (error) {
        next.close();
        throw error;
      }
      // Keep the displayed document until the UI accepts this result. A
      // superseded open, including one followed by an invalid file, cannot
      // invalidate the pages the reader is still looking at.
      documents.set(++revision, next);
      return { ...result, revision };
    }
    case "addFonts": {
      let added = 0;
      for (const bytes of args.buffers) added += core.addFont(bytes);
      fonts += added;
      const document = documents.get(args.revision);
      return {
        added,
        document: added && document ? { ...summary(document, performance.now()), revision: args.revision } : null,
      };
    }
    case "release":
      documents.get(args.revision)?.close();
      documents.delete(args.revision);
      return null;
    case "render":
      return documents.get(args.revision)?.pageSvg(args.page) ?? null;
    case "find":
      return documents.get(args.revision)?.find(args.query, args.limit) ?? null;
    default:
      throw new Error(`未知的文档操作：${type}`);
  }
}

// Serialize requests even when initialization is awaiting downloads. Rust's
// document handles and result buffer belong to one operation at a time.
let queue = Promise.resolve();
self.addEventListener("message", ({ data: { id, type, args } }) => {
  queue = queue.then(async () => {
    try {
      self.postMessage({ id, result: await handle(type, args) });
    } catch (error) {
      const fatal = error instanceof WebAssembly.RuntimeError || error.cause instanceof WebAssembly.RuntimeError;
      self.postMessage({ id, error: error.message || String(error), fatal });
    }
  });
});
