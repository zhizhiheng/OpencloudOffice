// Turns the SVG pages written by `oco render` into PNG images, using the
// headless Chromium that Playwright provides.
//
//   node tools/zh/rasterize.mjs out/ours/04-report-headings/page-1.svg ...
//
// Each PNG is written next to its SVG. The scale matches the 110 dpi that
// tools/zh/reference.py uses for the LibreOffice pages, so the two can be
// laid side by side.
import { createRequire } from "node:module";
import fs from "node:fs";
import path from "node:path";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");
const SCALE = 110 / 72;

const browser = await chromium.launch();
const page = await browser.newPage();
for (const file of process.argv.slice(2)) {
  const svg = fs.readFileSync(file, "utf8");
  const [, width, height] = svg.match(/viewBox="0 0 ([\d.]+) ([\d.]+)"/);
  await page.setViewportSize({ width: Math.round(width * SCALE), height: Math.round(height * SCALE) });
  await page.setContent(
    `<style>html,body{margin:0}svg{display:block;width:100vw;height:100vh}</style>${svg}`,
  );
  await page.screenshot({ path: file.replace(/\.svg$/, ".png") });
}
await browser.close();
