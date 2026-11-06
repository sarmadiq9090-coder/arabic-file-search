// Renders every mockup screen (7 screens x ar/en x light/dark) with Playwright.
//   node render.cjs <outDir> [deviceScaleFactor]
// Design tooling only; not part of the app or CI.
const { chromium } = require("playwright");
const path = require("path");
const fs = require("fs");

const SCREENS = ["main", "quick", "settings", "first", "empty", "nores", "indexing"];
const out = path.resolve(process.argv[2] || path.join(__dirname, "..", "shots"));
const dpr = Number(process.argv[3] || 1);
fs.mkdirSync(out, { recursive: true });

(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: dpr });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("request", (r) => { if (!r.url().startsWith("file:")) errors.push("network request: " + r.url()); });
  for (const s of SCREENS) for (const lang of ["ar", "en"]) for (const theme of ["light", "dark"]) {
    const url = "file://" + path.join(__dirname, "index.html") + `?screen=${s}&lang=${lang}&theme=${theme}&shot=1`;
    await page.goto(url);
    await page.evaluate(() => document.fonts.ready);
    const fonts = await page.evaluate(() => [...document.fonts].filter((f) => f.status === "loaded").length);
    if (fonts < 2) errors.push(`${s}-${lang}-${theme}: only ${fonts} font faces loaded`);
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth);
    if (overflow) errors.push(`${s}-${lang}-${theme}: horizontal overflow`);
    await page.screenshot({ path: path.join(out, `${s}-${lang}-${theme}.png`) });
  }
  await browser.close();
  if (errors.length) { console.error(errors.join("\n")); process.exit(1); }
  console.log(`rendered ${SCREENS.length * 4} screens to ${out} at ${dpr}x`);
})();
