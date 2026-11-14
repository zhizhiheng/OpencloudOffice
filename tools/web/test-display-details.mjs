// Exercises the display probe through the real WASM core, then checks
// pixels in Chromium. No reference application's rasterizer is needed to
// know which quadrant a mirrored four-colour picture must show.
import assert from "node:assert/strict";
import { readFile, mkdir } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { Oco } from "../../web/oco.js";

const root = resolve(fileURLToPath(import.meta.url), "../../..");
const oco = await Oco.load(await readFile(resolve(root, "web/oco.wasm")));
for (const font of JSON.parse(await readFile(resolve(root, "web/fonts/index.json"), "utf8"))) {
  oco.addFont(await readFile(resolve(root, "web/fonts", font)));
}
const document = oco.open(await readFile(resolve(root, "testdata/details/display-details.docx")), "display-details.docx");
const svgs = [];
try {
  assert.equal(document.pageCount, 3);
  for (const [page, footer] of [
    [0, "页 1 · 本节 1 页 · 全文 3 页 · 罗马页码 I"],
    [1, "页 iv · 本节 2 页 · 全文 3 页 · 罗马页码 IV"],
    [2, "页 v · 本节 2 页 · 全文 3 页 · 罗马页码 V"],
  ]) {
    assert(document.pageText(page).includes(footer), document.pageText(page));
    assert(document.pageText(page).includes("英文总页数 three"), "spelled-out field format in WASM");
    svgs.push(document.pageSvg(page));
  }
  assert(svgs[0].includes('fill="#365f91"'), "heading's theme shade");
  assert(svgs[0].includes('fill="#c0504d"'), "numbering's own theme colour");
  assert(svgs[0].includes('fill="#228833"'), "explicit RGB overrides the heading style");
} finally { document.close(); }

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined, headless: true, args: ["--no-sandbox"] });
try {
  const page = await browser.newPage({ viewport: { width: 1240, height: 1000 } });
  await page.setContent('<style>body{margin:0;padding:16px;background:#e9ebee;display:flex;gap:16px}canvas{width:390px;height:auto;align-self:flex-start;background:white;box-shadow:0 1px 4px #aaa}</style>');
  const sampled = await page.evaluate(async (svgs) => {
    const values = [];
    for (const [pageNumber, svg] of svgs.entries()) {
      const xml = new DOMParser().parseFromString(svg, "image/svg+xml");
      const [,, width, height] = xml.documentElement.getAttribute("viewBox").split(" ").map(Number);
      const canvas = document.createElement("canvas");
      canvas.width = Math.ceil(width * 2); canvas.height = Math.ceil(height * 2);
      document.body.append(canvas);
      const image = new Image();
      const url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
      try {
        image.src = url;
        await image.decode();
        const context = canvas.getContext("2d");
        context.drawImage(image, 0, 0, canvas.width, canvas.height);
        for (const picture of xml.querySelectorAll("image")) {
          // Cropped images have their own viewport. Reflection leaves the
          // viewport's bounds unchanged, so sample its visible top left.
          const holder = picture.parentElement.localName === "svg" && picture.parentElement !== xml.documentElement ? picture.parentElement : picture;
          const get = (key) => Number(holder.getAttribute(key));
          const x = (get("x") + get("width") / 4) * canvas.width / width;
          const y = (get("y") + get("height") / 4) * canvas.height / height;
          values.push({ page: pageNumber, width: get("width"), rgb: [...context.getImageData(Math.floor(x), Math.floor(y), 1, 1).data].slice(0, 3) });
        }
      } finally { URL.revokeObjectURL(url); }
    }
    return values;
  }, svgs);
  const body = sampled.filter((image) => image.page === 1 && image.width === 90).map((image) => image.rgb);
  assert.deepEqual(body, [[230, 40, 40], [30, 180, 80], [40, 90, 230], [245, 190, 25], [30, 180, 80]], "normal / H / V / HV / crop-then-H");
  const headers = sampled.filter((image) => image.width === 18);
  assert.equal(headers.length, 3);
  assert(headers.every((image) => image.rgb.join() === "30,180,80"), "mirrored picture in every inherited header");
  await mkdir(resolve(root, "out/display-details"), { recursive: true });
  await page.screenshot({ path: resolve(root, "out/display-details/overview.png"), fullPage: true });
  console.log("PASS three-page section fields, themed headings and numbering, four picture orientations, cropping and header pixels");
} finally { await browser.close(); }
