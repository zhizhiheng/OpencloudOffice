// Capture the real release viewer. No DOM or CSS substitutions are applied.
// Run tools/web/build.sh and python3 tools/releases/make_demo.py first.
// PLAYWRIGHT_MODULE and CHROMIUM_PATH may point to an existing installation.
import { createServer } from "node:http";
import { readFile, mkdir, writeFile } from "node:fs/promises";
import { extname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = resolve(fileURLToPath(import.meta.url), "../../..");
const web = resolve(process.env.OCO_WEB_DIR || resolve(root, "web"));
const out = resolve(root, "docs/images");
await mkdir(out, { recursive: true });
const mime = { ".html": "text/html", ".js": "text/javascript", ".json": "application/json", ".wasm": "application/wasm" };
const server = createServer(async (request, response) => {
  try {
    const pathname = decodeURIComponent(new URL(request.url, "http://local").pathname);
    const file = resolve(web, `.${pathname === "/" ? "/index.html" : pathname}`);
    if (!file.startsWith(`${web}/`)) { response.writeHead(403).end(); return; }
    const data = await readFile(file);
    response.writeHead(200, { "Content-Type": mime[extname(file)] || "application/octet-stream" }).end(data);
  } catch { response.writeHead(404).end(); }
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));
let browser;
try {
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined, headless: true, args: ["--no-sandbox"] });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1200 }, deviceScaleFactor: 1 });
  page.setDefaultTimeout(30000);
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${server.address().port}`, { waitUntil: "networkidle" });
  await page.waitForFunction(() => !document.querySelector("#open").disabled);
  await page.locator("#file").setInputFiles(resolve(root, "testdata/release/opencloudoffice-0.0.1.docx"));
  await page.waitForFunction(() => document.querySelector("#info").textContent.includes("opencloudoffice-0.0.1.docx") && document.querySelector("#desk").getAttribute("aria-busy") === "false");
  await page.locator('.page[data-page="0"][data-rendered]').waitFor();
  assert.equal(await page.locator(".page").count(), 2);
  while (parseFloat(await page.locator("#zoom-value").textContent()) > 100) await page.click("#zoom-out");
  await page.click("#info-toggle");
  await page.screenshot({ path: resolve(out, "viewer-document.png") });

  await page.click("#info-toggle");
  await page.click("#search-toggle");
  await page.locator("#search-query").fill("文档");
  await page.waitForFunction(() => document.querySelectorAll(".search-hit").length > 0);
  await page.screenshot({ path: resolve(out, "viewer-search.png") });
  const searchCount = await page.locator("#search-count").textContent();
  await page.click("#search-close");

  await page.locator("#page-number").fill("2");
  await page.locator("#page-number").press("Enter");
  await page.locator('.page[data-page="1"][data-rendered]').waitFor();
  await page.screenshot({ path: resolve(out, "viewer-footnotes.png") });
  assert.deepEqual(errors, []);
  await writeFile(resolve(out, "screenshots.json"), JSON.stringify({
    version: "0.0.1", browser: await browser.version(), viewport: { width: 1440, height: 1200 },
    zoom: await page.locator("#zoom-value").textContent(),
    document: "testdata/release/opencloudoffice-0.0.1.docx", pages: 2, searchQuery: "文档", searchCount,
    screenshots: ["viewer-document.png", "viewer-search.png", "viewer-footnotes.png"],
    method: "Playwright captures of the actual viewer and WASM output; no DOM or CSS substitutions.",
  }, null, 2) + "\n");
  console.log("3 screenshots captured; 2 pages; " + searchCount + "; no browser errors");
} finally {
  if (browser) await browser.close();
  await new Promise((done) => server.close(done));
}
