// The viewer: opens a .docx from a file, a drop or a URL, and shows its
// pages as the core lays them out. Everything happens in this page; the
// document goes nowhere.

import { Oco } from "./oco.js";

const $ = (id) => document.getElementById(id);
const desk = $("desk");
const pages = $("pages");
const status = $("status");
const bar = $("progress").firstElementChild;

/** CSS pixels per point at 100%. */
const PX_PER_PT = 96 / 72;
/** How many pages keep their drawing while out of sight. */
const KEEP_DRAWN = 12;

const NOT_SHOWN = {
  pictures: "图片",
  text_boxes: "文本框和形状",
  embedded_objects: "嵌入对象",
  notes: "脚注和尾注",
  equations: "公式",
  columns: "分栏",
  frames: "图文框",
  floating_tables: "浮动表格",
  comments: "批注",
};

let oco = null;
/** The open document: its core object, name, page sizes and timings. */
let current = null;
let zoom = 1;
let fitWidth = true;
let fontCount = 0;

function say(message, isError = false) {
  status.textContent = message;
  status.classList.toggle("error", isError);
}

function progress(fraction) {
  bar.style.width = fraction === null ? "0" : `${Math.round(fraction * 100)}%`;
}

async function download(url, onProgress) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}：HTTP ${response.status}`);
  const total = Number(response.headers.get("content-length")) || 0;
  if (!response.body || !onProgress) return new Uint8Array(await response.arrayBuffer());
  const chunks = [];
  let received = 0;
  for (const reader = response.body.getReader(); ; ) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    received += value.length;
    onProgress(received, total);
  }
  const bytes = new Uint8Array(received);
  let at = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, at);
    at += chunk.length;
  }
  return bytes;
}

/** Loads the core and the fonts listed in fonts/index.json, in the order listed. */
async function start() {
  say("正在加载排版内核…");
  oco = await Oco.load(new URL("oco.wasm", import.meta.url));

  let list = [];
  try {
    list = await (await fetch("fonts/index.json")).json();
  } catch {
    // No font list: the page still works once fonts are added by hand.
  }
  const sizes = new Array(list.length).fill(0);
  const fetched = new Array(list.length);
  let next = 0;
  let done = 0;
  const report = () => {
    const megabytes = (sizes.reduce((a, b) => a + b, 0) / 1048576).toFixed(0);
    say(`正在加载字体 ${done}/${list.length}（已下载 ${megabytes} MB，首次较慢，之后由浏览器缓存）`);
    progress(list.length ? done / list.length : null);
  };
  const worker = async () => {
    while (next < list.length) {
      const index = next++;
      try {
        fetched[index] = await download(`fonts/${list[index]}`, (received) => {
          sizes[index] = received;
          report();
        });
      } catch (error) {
        console.warn(error);
      }
      done++;
      report();
    }
  };
  await Promise.all(Array.from({ length: 4 }, worker));
  // Fonts are handed over in the listed order: which font stands in for a
  // missing one depends on it, and the result must not depend on the network.
  for (const bytes of fetched) {
    if (bytes) fontCount += oco.addFont(bytes);
  }
  progress(null);
  say(fontCount ? "" : "没有可用的字体。请点击“添加字体”选择字体文件，否则无法排版。", !fontCount);

  try {
    const samples = await (await fetch("samples/index.json")).json();
    for (const file of samples) $("samples").append(new Option(file.replace(/\.docx$/, ""), `samples/${file}`));
    $("samples").hidden = samples.length === 0;
  } catch {}

  const wanted = new URLSearchParams(location.search).get("file");
  if (wanted) await openUrl(wanted);
}

async function openUrl(url) {
  try {
    say(`正在下载 ${url} …`);
    const bytes = await download(url, (received, total) => progress(total ? received / total : null));
    progress(null);
    show(bytes, decodeURIComponent(new URL(url, location.href).pathname.split("/").pop() || url));
  } catch (error) {
    progress(null);
    say(`无法下载这个文档：${error.message}`, true);
  }
}

/** Opens the bytes of a document and puts its pages on the desk. */
function show(bytes, name) {
  if (!oco) return;
  let opened;
  const began = performance.now();
  try {
    opened = oco.open(bytes);
  } catch (error) {
    say(`无法打开“${name}”：${error.message}`, true);
    return;
  }
  current?.doc.close();
  current = { doc: opened, name, bytes: bytes.length, sizes: [], elapsed: 0 };
  arrange(began);
}

/** Lays the open document out and builds an empty sheet per page; pages are drawn as they come into view. */
function arrange(began = performance.now()) {
  const { doc } = current;
  const count = doc.pageCount;
  current.sizes = Array.from({ length: count }, (_, page) => doc.pageSize(page));
  current.elapsed = performance.now() - began;
  $("welcome").hidden = true;
  say("");

  watcher.disconnect();
  drawn.clear();
  pages.replaceChildren(
    ...current.sizes.map((_, page) => {
      const sheet = document.createElement("div");
      sheet.className = "page";
      sheet.dataset.page = page;
      sheet.setAttribute("aria-label", `第 ${page + 1} 页`);
      return sheet;
    }),
  );
  resize();
  for (const sheet of pages.children) watcher.observe(sheet);
  desk.scrollTop = 0;
  indicate();
  describe();
}

/** Sheets that hold a drawing, oldest first. */
const drawn = new Set();

const watcher = new IntersectionObserver(
  (entries) => {
    for (const entry of entries) {
      const sheet = entry.target;
      if (!entry.isIntersecting || !current) continue;
      if (!sheet.firstChild) sheet.innerHTML = current.doc.pageSvg(Number(sheet.dataset.page));
      // Most recently seen last.
      drawn.delete(sheet);
      drawn.add(sheet);
    }
    for (const sheet of drawn) {
      if (drawn.size <= KEEP_DRAWN) break;
      const box = sheet.getBoundingClientRect();
      const view = desk.getBoundingClientRect();
      if (box.bottom < view.top - view.height || box.top > view.bottom + view.height) {
        sheet.replaceChildren();
        drawn.delete(sheet);
      }
    }
  },
  { root: desk, rootMargin: "100% 0px" },
);

/** Sizes the sheets for the zoom in force. */
function resize() {
  if (!current) return;
  if (fitWidth) {
    const widest = Math.max(...current.sizes.map((size) => size.width), 1);
    const room = desk.clientWidth - 2 * parseFloat(getComputedStyle(desk).paddingLeft);
    zoom = Math.max(0.1, Math.min(room / (widest * PX_PER_PT), 2));
  }
  [...pages.children].forEach((sheet, page) => {
    sheet.style.width = `${current.sizes[page].width * PX_PER_PT * zoom}px`;
    sheet.style.height = `${current.sizes[page].height * PX_PER_PT * zoom}px`;
  });
  $("zoom-value").textContent = `${Math.round(zoom * 100)}%`;
  $("zoom-fit").setAttribute("aria-pressed", String(fitWidth));
}

function setZoom(value) {
  // Keep the same place in the document under the middle of the window.
  const before = (desk.scrollTop + desk.clientHeight / 2) / Math.max(desk.scrollHeight, 1);
  fitWidth = false;
  zoom = Math.max(0.1, Math.min(value, 5));
  resize();
  desk.scrollTop = before * desk.scrollHeight - desk.clientHeight / 2;
  indicate();
}

/** Shows which page is in the middle of the window. */
function indicate() {
  const sheets = pages.children;
  if (!current || sheets.length === 0) {
    $("page-indicator").textContent = "";
    return;
  }
  const middle = desk.getBoundingClientRect().top + desk.clientHeight / 2;
  let low = 0;
  let high = sheets.length - 1;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (sheets[mid].getBoundingClientRect().bottom < middle) low = mid + 1;
    else high = mid;
  }
  $("page-indicator").textContent = `第 ${low + 1} / ${sheets.length} 页`;
}

function element(tag, text, className) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
}

/** Fills the side panel: what the document is, and where it may differ from Word. */
function describe() {
  const panel = $("info");
  if (!current) return panel.replaceChildren();
  const info = current.doc.info();
  const facts = element("dl");
  const fact = (name, value) => facts.append(element("dt", name), element("dd", value));
  fact("文件", current.name);
  fact("大小", current.bytes < 1048576 ? `${Math.ceil(current.bytes / 1024)} KB` : `${(current.bytes / 1048576).toFixed(1)} MB`);
  fact("页数", String(info.pages));
  fact("读取并排版", `${current.elapsed < 10 ? current.elapsed.toFixed(1) : Math.round(current.elapsed)} 毫秒`);
  const nodes = [element("h2", "文档"), facts, element("h2", "与 Word 可能不同的地方")];

  if (info.notShown.length === 0 && info.missingFonts.length === 0 && info.warnings.length === 0) {
    nodes.push(element("p", "没有发现已知会造成差异的内容。", "fine"));
  }
  if (info.notShown.length) {
    const list = element("ul");
    for (const { kind, count } of info.notShown) list.append(element("li", `${NOT_SHOWN[kind] ?? kind}：${count} 处`));
    nodes.push(element("p", "下面这些内容还不能显示，它们所在位置之后的版面可能与 Word 不同：", "note"), list);
  }
  if (info.missingFonts.length) {
    const list = element("ul");
    for (const font of info.missingFonts) list.append(element("li", font));
    nodes.push(
      element("p", "下面这些字体没有，也没有字宽相同的替代字体。用到它们的文字换了字体，换行位置可能不同：", "note"),
      list,
      element("p", "如果本机有这些字体的文件，可以用“添加字体”加进来。"),
    );
  }
  if (info.warnings.length) {
    const list = element("ul");
    for (const warning of info.warnings) list.append(element("li", warning));
    nodes.push(element("p", "文件有损坏的部分，已忽略：", "note"), list);
  }
  panel.replaceChildren(...nodes);
}

// ---------------------------------------------------------------- controls

$("open").addEventListener("click", () => $("file").click());
$("file").addEventListener("change", async (event) => {
  const file = event.target.files[0];
  if (file) show(new Uint8Array(await file.arrayBuffer()), file.name);
  event.target.value = "";
});
$("samples").addEventListener("change", (event) => {
  if (event.target.value) openUrl(event.target.value);
});
$("url-form").addEventListener("submit", (event) => {
  event.preventDefault();
  if ($("url").value) openUrl($("url").value);
});
$("add-font").addEventListener("click", () => $("font-file").click());
$("font-file").addEventListener("change", async (event) => {
  let added = 0;
  for (const file of event.target.files) added += oco.addFont(new Uint8Array(await file.arrayBuffer()));
  event.target.value = "";
  fontCount += added;
  say(added ? `已添加 ${added} 个字体。` : "选中的文件不是可用的字体。", !added);
  // The core lays open documents out afresh; the sheets have to follow.
  if (added && current) arrange();
});
$("zoom-in").addEventListener("click", () => setZoom(zoom * 1.2));
$("zoom-out").addEventListener("click", () => setZoom(zoom / 1.2));
$("zoom-fit").addEventListener("click", () => {
  fitWidth = true;
  resize();
  indicate();
});
$("info-toggle").addEventListener("click", (event) => {
  const panel = $("info");
  panel.hidden = !panel.hidden;
  event.currentTarget.setAttribute("aria-pressed", String(!panel.hidden));
  resize();
});

let scheduled = false;
desk.addEventListener("scroll", () => {
  if (scheduled) return;
  scheduled = true;
  requestAnimationFrame(() => {
    scheduled = false;
    indicate();
  });
});
new ResizeObserver(() => {
  if (fitWidth) resize();
  indicate();
}).observe(desk);

window.addEventListener("keydown", (event) => {
  if (!(event.ctrlKey || event.metaKey)) return;
  if (event.key === "=" || event.key === "+") setZoom(zoom * 1.2);
  else if (event.key === "-") setZoom(zoom / 1.2);
  else if (event.key === "o") $("file").click();
  else return;
  event.preventDefault();
});

// Dropping a file anywhere opens it; a font file is added to the fonts.
window.addEventListener("dragover", (event) => {
  event.preventDefault();
  document.body.classList.add("dragging");
});
window.addEventListener("dragleave", (event) => {
  if (!event.relatedTarget) document.body.classList.remove("dragging");
});
window.addEventListener("drop", async (event) => {
  event.preventDefault();
  document.body.classList.remove("dragging");
  for (const file of event.dataTransfer.files) {
    const bytes = new Uint8Array(await file.arrayBuffer());
    if (/\.(ttf|otf|ttc)$/i.test(file.name)) {
      fontCount += oco.addFont(bytes);
      if (current) arrange();
    } else {
      show(bytes, file.name);
    }
  }
});

start().catch((error) => {
  progress(null);
  say(`启动失败：${error.message}`, true);
});
