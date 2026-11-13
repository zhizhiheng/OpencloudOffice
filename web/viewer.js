// The UI owns page placeholders and navigation; the worker owns the core.
import { ViewerClient } from "./viewer-client.js";

const $ = (id) => document.getElementById(id);
const desk = $("desk");
const pages = $("pages");
const status = $("status");
const progressBar = $("progress");
const PX_PER_PT = 96 / 72;
const KEEP_DRAWN = 12;
const SEARCH_LIMIT = 1000;
const NOT_SHOWN = {
  pictures: "图片", text_boxes: "文本框和形状", embedded_objects: "嵌入对象",
  notes: "脚注和尾注", equations: "公式", columns: "分栏", frames: "图文框",
  floating_tables: "浮动表格", comments: "批注",
};

let current = null;
let zoom = 1;
let fitWidth = true;
let ready = false;
let broken = false;
let busy = false;
let updatingFonts = false;
let openSequence = 0;
let downloadController = null;
let viewSequence = 0;
const visible = new Set();
const drawn = new Set();
let rendering = false;
let searchSequence = 0;
let searchTimer;
let matches = [];
let matchIndex = -1;
let searchLimited = false;
let searchPending = false;

function say(message, isError = false) {
  status.textContent = message;
  status.classList.toggle("error", isError);
}

function progress(fraction) {
  progressBar.classList.toggle("busy", fraction === undefined);
  progressBar.firstElementChild.style.width = fraction === undefined ? "" : fraction === null ? "0" : `${Math.round(fraction * 100)}%`;
}

function controls() {
  $("open").disabled = $("open-url").disabled = $("samples").disabled = !ready || broken || updatingFonts;
  $("add-font").disabled = !ready || busy || broken;
  const empty = !current?.sizes.length;
  for (const id of ["zoom-in", "zoom-out", "zoom-fit", "info-toggle", "page-number", "search-toggle"]) {
    $(id).disabled = empty || broken;
  }
  indicate();
}

function setBusy(value) {
  busy = value;
  desk.setAttribute("aria-busy", String(value));
  progress(value ? undefined : null);
  controls();
}

const client = new ViewerClient({
  onProgress({ done, total, bytes }) {
    say(`正在加载字体 ${done}/${total}（${(bytes / 1048576).toFixed(0)} MB）…`);
    progress(total ? done / total : null);
  },
  onFailure(error) {
    broken = true;
    setBusy(false);
    say(error.message, true);
  },
});

async function start() {
  say("正在加载排版内核…");
  const { fonts, failed } = await client.request("init");
  ready = true;
  progress(null);
  controls();
  say(!fonts ? "没有可用的字体，请点击“添加字体”。" : failed.length ? `${failed.length} 个字体资源未加载，可在文档信息中查看缺失字体。` : "", !fonts);
  try {
    const response = await fetch("samples/index.json");
    if (response.ok) {
      const samples = await response.json();
      for (const file of samples) $("samples").append(new Option(file.replace(/\.docx$/, ""), `samples/${file}`));
      $("samples").hidden = samples.length === 0;
    }
  } catch { /* Samples are optional. */ }
  const wanted = new URLSearchParams(location.search).get("file");
  if (wanted) await openUrl(wanted);
}

function beginOpen() {
  downloadController?.abort();
  downloadController = null;
  const sequence = ++openSequence;
  setBusy(true);
  return sequence;
}

async function openBytes(bytes, name, sequence) {
  if (sequence !== openSequence) return;
  const size = bytes.byteLength;
  say(`正在排版“${name}”…`);
  const document = await client.open(bytes, name);
  if (sequence !== openSequence) {
    void client.request("release", { revision: document.revision }).catch(() => {});
    return;
  }
  if (current) void client.request("release", { revision: current.revision }).catch(() => {});
  current = { ...document, name, bytes: size };
  arrange();
  say(current.sizes.length ? "" : "文档中没有可显示的页面。");
}

async function openFile(file) {
  if (!ready || broken || updatingFonts) return;
  const sequence = beginOpen();
  $("samples").value = "";
  try {
    say(`正在读取“${file.name}”…`);
    await openBytes(await file.arrayBuffer(), file.name, sequence);
  } catch (error) {
    if (sequence === openSequence) say(`无法打开“${file.name}”：${error.message}`, true);
  } finally {
    if (sequence === openSequence) setBusy(false);
  }
}

async function openUrl(address) {
  if (!ready || broken || updatingFonts) return;
  const sequence = beginOpen();
  const controller = downloadController = new AbortController();
  try {
    const url = new URL(address, location.href);
    if (!["http:", "https:"].includes(url.protocol)) throw new Error("请使用 HTTP 或 HTTPS 文档网址。");
    say("正在下载文档…");
    const response = await fetch(url, { signal: controller.signal });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    let name = url.pathname.split("/").pop() || "文档";
    try { name = decodeURIComponent(name); } catch { /* A malformed URL escape is still a usable file name. */ }
    await openBytes(await response.arrayBuffer(), name, sequence);
  } catch (error) {
    if (sequence === openSequence && error.name !== "AbortError") say(`无法打开文档：${error.message}`, true);
  } finally {
    if (sequence === openSequence) {
      downloadController = null;
      setBusy(false);
    }
  }
}

async function addFonts(files) {
  if (!ready || busy || broken || !files.length) return;
  updatingFonts = true;
  const sequence = beginOpen();
  const position = readingPosition();
  try {
    say("正在添加字体并重新排版…");
    const buffers = await Promise.all(files.map((file) => file.arrayBuffer()));
    if (sequence !== openSequence) return;
    const result = await client.addFonts(buffers, current?.revision);
    if (sequence !== openSequence) return;
    if (result.document && current) {
      current = { ...current, ...result.document };
      arrange(position);
    }
    say(result.added ? `已添加 ${result.added} 个字体${current ? "，文档已重新排版" : ""}。` : "选中的文件不是可用的字体。", !result.added);
  } catch (error) {
    if (sequence === openSequence) say(`无法添加字体：${error.message}`, true);
  } finally {
    updatingFonts = false;
    if (sequence === openSequence) setBusy(false);
  }
}

function arrange(position = null) {
  viewSequence++;
  watcher.disconnect();
  visible.clear();
  drawn.clear();
  resetSearch();
  $("welcome").hidden = true;
  const fragment = document.createDocumentFragment();
  for (let page = 0; page < current.sizes.length; page++) {
    const sheet = document.createElement("div");
    sheet.className = "page";
    sheet.dataset.page = page;
    sheet.setAttribute("aria-label", `第 ${page + 1} 页`);
    const art = document.createElement("div");
    art.className = "page-art";
    sheet.append(art);
    fragment.append(sheet);
  }
  pages.replaceChildren(fragment);
  resize(position);
  if (!position) desk.scrollTop = 0;
  for (const sheet of pages.children) watcher.observe(sheet);
  controls();
  describe();
  if (!$("search-bar").hidden && $("search-query").value.trim()) scheduleSearch();
}

const watcher = new IntersectionObserver((entries) => {
  for (const entry of entries) {
    if (!entry.target.isConnected) continue;
    if (entry.isIntersecting) visible.add(entry.target);
    else visible.delete(entry.target);
  }
  evict();
  void drawVisible();
}, { root: desk, rootMargin: "100% 0px" });

// Only one SVG request is in flight. Fast scrolling does not queue every
// page passed along the way in front of the page the reader actually wants.
async function drawVisible() {
  if (rendering || broken) return;
  rendering = true;
  try {
    while (current && !broken) {
      const middle = desk.getBoundingClientRect().top + desk.clientHeight / 2;
      const sheet = [...visible].filter((s) => s.isConnected && !s.hasAttribute("data-rendered") && !s.hasAttribute("data-failed"))
        .sort((a, b) => Math.abs(a.getBoundingClientRect().top - middle) - Math.abs(b.getBoundingClientRect().top - middle))[0];
      if (!sheet) break;
      const sequence = viewSequence;
      const revision = current.revision;
      try {
        const svg = await client.request("render", { revision, page: Number(sheet.dataset.page) });
        if (sequence !== viewSequence || !sheet.isConnected) continue;
        if (svg === null) break; // A newer document is being opened in the worker.
        if (!visible.has(sheet)) continue;
        sheet.querySelector(".page-art").innerHTML = svg;
        sheet.dataset.rendered = "";
        drawn.delete(sheet);
        drawn.add(sheet);
        evict();
      } catch (error) {
        if (sequence !== viewSequence || !sheet.isConnected) continue;
        sheet.dataset.failed = "";
        const notice = element("div", `此页显示失败：${error.message} `, "page-error");
        const retry = element("button", "重试");
        retry.addEventListener("click", () => {
          notice.remove();
          delete sheet.dataset.failed;
          void drawVisible();
        });
        notice.append(retry);
        sheet.append(notice);
      }
    }
  } finally { rendering = false; }
}

function selected(sheet) {
  const selection = window.getSelection();
  if (!selection || selection.isCollapsed) return false;
  for (let i = 0; i < selection.rangeCount; i++) {
    if (selection.getRangeAt(i).intersectsNode(sheet)) return true;
  }
  return false;
}

function evict() {
  for (const sheet of drawn) {
    if (drawn.size <= KEEP_DRAWN) break;
    if (visible.has(sheet) || selected(sheet)) continue;
    sheet.querySelector(".page-art").replaceChildren();
    delete sheet.dataset.rendered;
    drawn.delete(sheet);
  }
}

function pageAtMiddle() {
  const sheets = pages.children;
  if (!sheets.length) return 0;
  const middle = desk.getBoundingClientRect().top + desk.clientHeight / 2;
  let low = 0, high = sheets.length - 1;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (sheets[mid].getBoundingClientRect().bottom < middle) low = mid + 1;
    else high = mid;
  }
  return low;
}

function readingPosition() {
  if (!current?.sizes.length) return null;
  const page = pageAtMiddle();
  const box = pages.children[page].getBoundingClientRect();
  return { page, fraction: Math.max(0, Math.min(1, (desk.getBoundingClientRect().top + desk.clientHeight / 2 - box.top) / box.height)) };
}

function restorePosition(position) {
  const sheet = pages.children[Math.min(position.page, pages.children.length - 1)];
  if (!sheet) return;
  const box = sheet.getBoundingClientRect();
  desk.scrollTop += box.top - desk.getBoundingClientRect().top + position.fraction * box.height - desk.clientHeight / 2;
}

function resize(position = readingPosition()) {
  if (!current) return;
  if (fitWidth) {
    const widest = current.sizes.reduce((width, size) => Math.max(width, size.width), 1);
    const style = getComputedStyle(desk);
    const room = desk.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
    zoom = Math.max(0.1, Math.min(room / (widest * PX_PER_PT), 2));
  }
  [...pages.children].forEach((sheet, page) => {
    sheet.style.width = `${current.sizes[page].width * PX_PER_PT * zoom}px`;
    sheet.style.height = `${current.sizes[page].height * PX_PER_PT * zoom}px`;
  });
  $("zoom-value").textContent = `${Math.round(zoom * 100)}%`;
  $("zoom-fit").setAttribute("aria-pressed", String(fitWidth));
  if (position) restorePosition(position);
  indicate();
}

function setZoom(value) {
  if (!current) return;
  const position = readingPosition();
  fitWidth = false;
  zoom = Math.max(0.1, Math.min(value, 5));
  resize(position);
}

function goToPage(page, y = 0) {
  const sheet = pages.children[page];
  if (!sheet) return;
  desk.scrollTop += sheet.getBoundingClientRect().top - desk.getBoundingClientRect().top + y * PX_PER_PT * zoom - 20;
  indicate();
}

function indicate() {
  const count = current?.sizes.length ?? 0;
  const page = count ? pageAtMiddle() + 1 : 1;
  if (document.activeElement !== $("page-number")) $("page-number").value = page;
  $("page-number").max = Math.max(1, count);
  $("page-total").textContent = `/ ${count} 页`;
  $("page-indicator").textContent = count ? `第 ${page} / ${count} 页` : "";
  $("page-prev").disabled = broken || page <= 1;
  $("page-next").disabled = broken || page >= count;
}

function resetSearch() {
  clearTimeout(searchTimer);
  searchSequence++;
  searchPending = false;
  matches = [];
  matchIndex = -1;
  searchLimited = false;
  for (const layer of pages.querySelectorAll(".search-highlights")) layer.remove();
  updateSearchCount();
}

function updateSearchCount(message = "") {
  $("search-count").textContent = message || (matches.length ? `${matchIndex + 1} / ${matches.length}${searchLimited ? "+" : ""}` : "");
  $("search-count").title = searchLimited ? `显示前 ${SEARCH_LIMIT} 个匹配` : "";
  $("search-prev").disabled = $("search-next").disabled = !matches.length || searchPending;
}

function toggleSearch(open) {
  const position = readingPosition();
  $("search-bar").hidden = !open;
  $("search-toggle").setAttribute("aria-expanded", String(open));
  if (position) restorePosition(position);
  if (open) {
    $("search-query").focus();
    $("search-query").select();
    scheduleSearch();
  } else {
    resetSearch();
    $("search-toggle").focus();
  }
}

function scheduleSearch() {
  resetSearch();
  if (!current || $("search-bar").hidden || !$("search-query").value.trim()) return;
  searchPending = true;
  updateSearchCount("正在查找…");
  searchTimer = setTimeout(() => void runSearch(), 150);
}

async function runSearch() {
  clearTimeout(searchTimer);
  if (!current || $("search-bar").hidden) return;
  const query = $("search-query").value.trim();
  if (!query) return resetSearch();
  const sequence = ++searchSequence;
  const revision = current.revision;
  searchPending = true;
  updateSearchCount("正在查找…");
  try {
    const result = await client.request("find", { revision, query, limit: SEARCH_LIMIT + 1 });
    if (sequence !== searchSequence || current?.revision !== revision) return;
    searchPending = false;
    if (result === null) return updateSearchCount();
    matches = result.slice(0, SEARCH_LIMIT);
    searchLimited = result.length > SEARCH_LIMIT;
    if (!matches.length) return updateSearchCount("未找到匹配");
    const near = matches.findIndex((match) => match.page >= pageAtMiddle());
    matchIndex = near < 0 ? 0 : near;
    paintMatches();
    revealMatch();
  } catch (error) {
    if (sequence !== searchSequence) return;
    searchPending = false;
    updateSearchCount("查找失败");
    say(error.message, true);
  }
}

function paintMatches() {
  for (const layer of pages.querySelectorAll(".search-highlights")) layer.remove();
  const layers = new Map();
  matches.forEach((match, index) => {
    const sheet = pages.children[match.page];
    if (!sheet) return;
    let layer = layers.get(match.page);
    if (!layer) {
      layer = element("div", undefined, "search-highlights");
      layer.setAttribute("aria-hidden", "true");
      sheet.append(layer);
      layers.set(match.page, layer);
    }
    const size = current.sizes[match.page];
    for (const [x, y, width, height] of match.boxes) {
      const hit = element("span", undefined, "search-hit");
      hit.dataset.match = index;
      Object.assign(hit.style, { left: `${100 * x / size.width}%`, top: `${100 * y / size.height}%`, width: `${100 * width / size.width}%`, height: `${100 * height / size.height}%` });
      layer.append(hit);
    }
  });
}

function revealMatch() {
  for (const hit of pages.querySelectorAll(".search-hit")) hit.classList.toggle("active", Number(hit.dataset.match) === matchIndex);
  const match = matches[matchIndex];
  if (match) goToPage(match.page, match.boxes[0]?.[1] ?? 0);
  updateSearchCount();
}

function moveMatch(step) {
  if (searchPending) return void runSearch();
  if (!matches.length) return;
  matchIndex = (matchIndex + step + matches.length) % matches.length;
  revealMatch();
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
  const info = current.info;
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

$("open").addEventListener("click", () => $("file").click());
$("file").addEventListener("change", (event) => {
  const file = event.target.files[0];
  event.target.value = "";
  if (file) void openFile(file);
});
$("samples").addEventListener("change", (event) => { if (event.target.value) void openUrl(event.target.value); });
$("url-form").addEventListener("submit", (event) => {
  event.preventDefault();
  if ($("url").value) { $("samples").value = ""; void openUrl($("url").value); }
});
$("add-font").addEventListener("click", () => $("font-file").click());
$("font-file").addEventListener("change", (event) => {
  const files = [...event.target.files];
  event.target.value = "";
  void addFonts(files);
});
$("zoom-in").addEventListener("click", () => setZoom(zoom * 1.2));
$("zoom-out").addEventListener("click", () => setZoom(zoom / 1.2));
$("zoom-fit").addEventListener("click", () => { fitWidth = true; resize(); });
$("info-toggle").addEventListener("click", (event) => {
  const position = readingPosition();
  $("info").hidden = !$("info").hidden;
  event.currentTarget.setAttribute("aria-pressed", String(!$("info").hidden));
  resize(position);
});
$("page-form").addEventListener("submit", (event) => {
  event.preventDefault();
  if (!current?.sizes.length) return;
  const page = Math.max(1, Math.min(current.sizes.length, Math.floor(Number($("page-number").value) || 1)));
  $("page-number").value = page;
  $("page-number").blur();
  goToPage(page - 1);
});
$("page-prev").addEventListener("click", () => goToPage(pageAtMiddle() - 1));
$("page-next").addEventListener("click", () => goToPage(pageAtMiddle() + 1));
$("search-toggle").addEventListener("click", () => toggleSearch($("search-bar").hidden));
$("search-close").addEventListener("click", () => toggleSearch(false));
$("search-query").addEventListener("input", scheduleSearch);
$("search-bar").addEventListener("submit", (event) => { event.preventDefault(); moveMatch(1); });
$("search-query").addEventListener("keydown", (event) => {
  if (event.key === "Enter" && event.shiftKey) { event.preventDefault(); moveMatch(-1); }
});
$("search-prev").addEventListener("click", () => moveMatch(-1));
$("search-next").addEventListener("click", () => moveMatch(1));

let scheduled = false;
desk.addEventListener("scroll", () => {
  if (scheduled) return;
  scheduled = true;
  requestAnimationFrame(() => { scheduled = false; indicate(); evict(); });
});
new ResizeObserver(() => { if (fitWidth) resize(); else indicate(); }).observe(desk);
document.addEventListener("selectionchange", evict);

window.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && !$("search-bar").hidden) { event.preventDefault(); toggleSearch(false); return; }
  if (!(event.ctrlKey || event.metaKey) || event.isComposing) return;
  const key = event.key.toLowerCase();
  if (key === "o" && ready && !broken && !updatingFonts) $("file").click();
  else if (key === "f" && current && !broken) toggleSearch(true);
  else if (event.target.closest("input, textarea, [contenteditable]")) return;
  else if (current && (key === "=" || key === "+")) setZoom(zoom * 1.2);
  else if (current && key === "-") setZoom(zoom / 1.2);
  else return;
  event.preventDefault();
});

window.addEventListener("dragover", (event) => { event.preventDefault(); document.body.classList.add("dragging"); });
window.addEventListener("dragleave", (event) => { if (!event.relatedTarget) document.body.classList.remove("dragging"); });
window.addEventListener("drop", async (event) => {
  event.preventDefault();
  document.body.classList.remove("dragging");
  if (!ready || broken) { say("排版内核正在加载，请稍后打开文件。"); return; }
  const files = [...event.dataTransfer.files];
  const fonts = files.filter((file) => /\.(ttf|otf|ttc)$/i.test(file.name));
  if (fonts.length) await addFonts(fonts);
  const document = files.find((file) => !/\.(ttf|otf|ttc)$/i.test(file.name));
  if (document) await openFile(document);
});

controls();
start().catch((error) => { progress(null); say(`启动失败：${error.message}`, true); });
