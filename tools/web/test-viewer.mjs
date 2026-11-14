// Browser integration tests against the real WASM build. Needs Playwright
// and Chromium: npm install --no-save playwright && npx playwright install chromium
// Run after tools/web/build.sh: node tools/web/test-viewer.mjs
// PLAYWRIGHT_MODULE and CHROMIUM_PATH may point at an existing installation.
import assert from "node:assert/strict";
import { readFile, mkdir } from "node:fs/promises";
import { createServer } from "node:http";
import { extname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { Oco } from "../../web/oco.js";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = resolve(fileURLToPath(import.meta.url), "../../..");
const web = resolve(root, "web");
await readFile(resolve(web, "oco.wasm")); // A missing build must fail, never skip.
const mime = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm", ".json": "application/json" };
let releaseSlow;
let slowStarted;
const slowRequest = new Promise((resolve) => { slowStarted = resolve; });
const slowResponse = new Promise((resolve) => { releaseSlow = resolve; });
const server = createServer(async (request, response) => {
  try {
    const pathname = decodeURIComponent(new URL(request.url, "http://local").pathname);
    if (pathname === "/slow.md") {
      slowStarted();
      await slowResponse;
      response.writeHead(200, { "Content-Type": "text/plain" }).end("OLD_DOWNLOAD");
      return;
    }
    const file = resolve(web, `.${pathname === "/" ? "/index.html" : pathname}`);
    if (!file.startsWith(`${web}/`)) { response.writeHead(403).end(); return; }
    let data = await readFile(file);
    if (pathname === "/viewer-worker.js") {
      // Hold only the background worker, to check that the UI still responds
      // while a document operation is running, without timing a particular CPU.
      data = Buffer.from(data.toString().replace('case "open": {', `case "open": {
        if (args.name === "blocked.md") {
          self.postMessage({ type: "test-blocked" });
          const until = performance.now() + 1200;
          while (performance.now() < until) {}
        }`));
    }
    response.writeHead(200, { "Content-Type": mime[extname(file)] || "application/octet-stream" }).end(data);
  } catch { response.writeHead(404).end(); }
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const base = `http://127.0.0.1:${server.address().port}`;
let browser;
try {
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined, headless: true, args: ["--no-sandbox"] });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, permissions: ["clipboard-read", "clipboard-write"] });
  const page = await context.newPage();
  page.setDefaultTimeout(20000);
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.addInitScript(() => {
    const NativeWorker = window.Worker;
    window.workerCount = 0;
    window.Worker = class extends NativeWorker {
      constructor(...args) {
        super(...args);
        window.workerCount++;
        this.addEventListener("message", ({ data }) => { if (data.type === "test-blocked") window.workerBlocked = true; });
      }
    };
  });
  const loaded = async (name) => {
    await page.waitForFunction((name) => document.querySelector("#info").textContent.includes(name) && document.querySelector("#desk").getAttribute("aria-busy") === "false", name);
    await page.locator(".page[data-rendered]").first().waitFor();
  };
  const upload = async (name, text) => {
    await page.locator("#file").setInputFiles({ name, mimeType: "text/plain", buffer: Buffer.from(text) });
    await loaded(name);
  };
  const jump = async (number) => {
    await page.locator("#page-number").fill(String(number));
    await page.locator("#page-number").press("Enter");
    await page.locator(`.page[data-page="${number - 1}"][data-rendered]`).waitFor();
  };

  await page.goto(base, { waitUntil: "networkidle" });
  await page.waitForFunction(() => !document.querySelector("#open").disabled);
  assert.equal(await page.evaluate(() => window.workerCount), 1);
  const longText = Array.from({ length: 1200 }, (_, i) => i === 1199 ? "RemoteNeedle" : `Line ${i}: Preview content`).join("\n");
  await upload("long.md", longText);
  const count = await page.locator(".page").count();
  assert(count > 12);
  assert.equal(await page.locator(`.page[data-page="${count - 1}"] svg`).count(), 0);
  // Moving the core and batching fonts must not change the drawing.
  const reference = await Oco.load(await readFile(resolve(web, "oco.wasm")));
  for (const font of JSON.parse(await readFile(resolve(web, "fonts/index.json"), "utf8"))) {
    reference.addFont(await readFile(resolve(web, "fonts", font)));
  }
  const referenceDoc = reference.open(Buffer.from(longText), "long.md");
  assert.equal(referenceDoc.pageCount, count);
  await page.locator('.page[data-page="0"][data-rendered]').waitFor();
  assert(await page.evaluate((svg) => {
    const expected = new DOMParser().parseFromString(svg, "image/svg+xml").documentElement;
    return document.querySelector('.page[data-page="0"] .page-art > svg').isEqualNode(expected);
  }, referenceDoc.pageSvg(0)), "worker drawing must equal the synchronous SDK drawing");
  referenceDoc.close();
  console.log("PASS background and synchronous SDK drawings are identical");

  await page.keyboard.press("Control+f");
  await page.locator("#search-query").fill("remoteneedle");
  await page.waitForFunction(() => document.querySelector("#search-count").textContent === "1 / 1");
  await page.locator(`.page[data-page="${count - 1}"][data-rendered]`).waitFor();
  assert.equal(await page.locator(`.page[data-page="${count - 1}"] .search-hit.active`).count(), 1);
  const hit = await page.locator(".search-hit.active").boundingBox();
  const desk = await page.locator("#desk").boundingBox();
  assert(hit.y >= desk.y && hit.y < desk.y + desk.height);
  await page.locator("#search-query").fill("Preview");
  await page.waitForFunction(() => document.querySelector("#search-count").textContent.endsWith("/ 1000+"));
  await page.locator("#search-query").fill("absent-query");
  await page.waitForFunction(() => document.querySelector("#search-count").textContent === "未找到匹配");
  assert.equal(await page.locator(".search-hit").count(), 0);
  await page.keyboard.press("Escape");
  console.log(`PASS search across ${count} pages, highlights, result cap and clearing`);

  await jump(8);
  const before = await page.locator("#page-number").inputValue();
  await page.click("#zoom-in");
  assert.equal(await page.locator("#page-number").inputValue(), before);
  await page.click("#info-toggle");
  assert.equal(await page.locator("#page-number").inputValue(), before);
  await page.click("#zoom-fit");
  assert.equal(await page.locator("#page-number").inputValue(), before);
  await page.click("#info-toggle");
  await page.click("#page-next");
  assert.equal(Number(await page.locator("#page-number").inputValue()), Number(before) + 1);
  console.log("PASS page navigation and stable reading position when zooming");

  for (let number = 1; number <= count; number++) await jump(number);
  assert(await page.locator(".page[data-rendered]").count() <= 12);
  await jump(1);
  assert.equal(await page.evaluate(() => window.find("Preview")), true);
  await page.keyboard.press("Control+c");
  assert.equal(await page.evaluate(() => navigator.clipboard.readText()), "Preview");
  console.log("PASS bounded page cache, revisiting evicted pages and copying text");

  await page.locator("#file").setInputFiles({ name: "blocked.md", mimeType: "text/plain", buffer: Buffer.from("Stale document") });
  await page.waitForFunction(() => window.workerBlocked);
  assert(await page.locator("#progress > div").evaluate((bar) => bar.getBoundingClientRect().width > 0));
  const started = Date.now();
  await page.click("#info-toggle");
  await page.evaluate(() => new Promise(requestAnimationFrame));
  assert(Date.now() - started < 1000, "UI must respond while the worker is occupied");
  // Supersede an in-flight successful open with a damaged file. The previous
  // displayed document must remain usable, including its evicted pages.
  await page.locator("#file").setInputFiles({ name: "broken.docx", mimeType: "application/octet-stream", buffer: Buffer.from("bad zip") });
  await page.waitForFunction(() => document.querySelector("#status").textContent.includes("无法打开“broken.docx”"));
  assert((await page.locator("#info").textContent()).includes("long.md"));
  await jump(count);
  await jump(1);
  console.log("PASS UI responsiveness and transactional file switching");

  await page.locator("#url").fill(`${base}/slow.md`);
  await page.click("#open-url");
  await slowRequest;
  await upload("latest.md", "Latest file wins\nHello World");
  releaseSlow();
  await page.waitForTimeout(150);
  assert((await page.locator("#info").textContent()).includes("latest.md"));
  await page.locator("#url").fill(`${base}/does-not-exist.md`);
  await page.click("#open-url");
  await page.waitForFunction(() => document.querySelector("#status").textContent.includes("HTTP 404"));
  assert((await page.locator("#info").textContent()).includes("latest.md"));
  console.log("PASS stale downloads and HTTP errors preserve the chosen file");

  const fontFiles = JSON.parse(await readFile(resolve(web, "fonts/index.json"), "utf8"));
  assert(fontFiles.length, "At least one built font is required for these tests");
  await page.locator("#font-file").setInputFiles(resolve(web, "fonts", fontFiles[0]));
  await page.waitForFunction(() => document.querySelector("#status").textContent.includes("已添加") && document.querySelector("#desk").getAttribute("aria-busy") === "false");
  assert((await page.locator("#info").textContent()).includes("latest.md"));
  await page.keyboard.press("Control+f");
  await page.locator("#search-query").fill("Hello World");
  await page.waitForFunction(() => document.querySelector("#search-count").textContent === "1 / 1");
  console.log("PASS font addition, re-layout and subsequent search");

  await page.keyboard.press("Escape");
  await page.selectOption("#samples", "samples/07-regulation.docx");
  await loaded("07-regulation.docx");
  await page.keyboard.press("Control+f");
  await page.locator("#search-query").fill("文档");
  await page.waitForFunction(() => /\d+ \/ \d+/.test(document.querySelector("#search-count").textContent));
  await mkdir(resolve(root, "out/viewer-tests"), { recursive: true });
  await page.screenshot({ path: resolve(root, "out/viewer-tests/desktop.png") });
  if (await page.locator("#info").isVisible()) await page.click("#info-toggle");
  await page.setViewportSize({ width: 390, height: 844 });
  await page.waitForTimeout(150);
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth));
  await page.screenshot({ path: resolve(root, "out/viewer-tests/mobile.png") });
  assert.deepEqual(errors, []);
  console.log("PASS DOCX, Chinese search, mobile layout and no uncaught page errors");
} finally {
  releaseSlow();
  if (browser) await browser.close();
  server.closeAllConnections();
  await new Promise((resolve) => server.close(resolve));
}
