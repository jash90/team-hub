// Renders the two link-preview images, assets/og-en.png and assets/og-pl.png
// (1200 x 630), from a small HTML card that uses the page's own stylesheet,
// font and hero screenshot. Not part of serving the page; run it by hand when
// the headline or the hero screenshot changes.
//
//   python3 -m http.server 8198          (in landing/; not Caddy: its policy
//                                          forbids the card's inline style)
//   node tools/og-image.js               (needs Playwright: npm i playwright)
//
const path = require('path');
const { chromium } = require('playwright');

const base = process.argv[2] || 'http://127.0.0.1:8198/';
const assets = base + 'assets/';
const out = path.join(__dirname, '..', 'assets');

// Keep these in step with the <h1> and the kicker of the two pages.
const cards = {
  en: ['Self-hosted issue tracker', 'The issue says Done.', 'The pull request is still open.'],
  pl: ['Tracker zadań na własnym serwerze', 'Zadanie ma status Done.', 'Pull request wciąż jest otwarty.'],
};

const card = (lang, [kicker, claim, fact]) => `<!doctype html>
<html lang="${lang}">
<head>
<meta charset="utf-8">
<link rel="stylesheet" href="${assets}site.css">
<style>
  body { width: 1200px; height: 630px; overflow: hidden; position: relative; background: #fbfaf7; }
  .in { padding: 56px 64px 0; }
  .row { display: flex; align-items: center; justify-content: space-between; }
  .brand { font-size: 26px; gap: 12px; }
  .brand img { width: 34px; height: 34px; }
  .kicker { font-size: 17px; }
  h1 { margin-top: 38px; font-size: 74px; line-height: 1.02; }
  .shot { position: absolute; left: 64px; top: 366px; width: 1300px; height: 400px; overflow: hidden;
    border: 1px solid #d9d6cc; border-radius: 14px; box-shadow: 0 24px 60px -24px rgb(23 24 28 / .35); }
  /* The offsets put the board's filter row and first cards in the window. */
  .shot img { width: 1640px; max-width: none; margin: -138px 0 0 -272px; }
</style>
</head>
<body>
<div class="in">
  <div class="row">
    <span class="brand"><img src="${assets}favicon.svg" alt="">team-hub</span>
    <span class="kicker">${kicker}</span>
  </div>
  <h1><span class="claim">${claim}</span><span>${fact}</span></h1>
</div>
<div class="shot"><img src="${assets}img/board-light-1600.webp" alt=""></div>
</body>
</html>`;

(async () => {
  const browser = await chromium.launch();
  for (const [lang, words] of Object.entries(cards)) {
    const page = await browser.newPage({ viewport: { width: 1200, height: 630 }, colorScheme: 'light' });
    // Start on a page of the site, so that the font is same-origin for the card.
    await page.goto(base + '404.html');
    await page.setContent(card(lang, words), { waitUntil: 'networkidle' });
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({ path: path.join(out, `og-${lang}.png`) });
    await page.close();
    console.log(`assets/og-${lang}.png`);
  }
  await browser.close();
})();
