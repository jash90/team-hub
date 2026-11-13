// Checks the landing page in a real browser and prints one PASS or FAIL line
// per check. Not part of serving the page; run it after changing the page.
//
//   python3 -m http.server 8198      (in landing/), or Caddy with the Caddyfile,
//                                     which also puts the Content-Security-Policy to the test
//   node tools/verify.js [base URL]  (needs Playwright: npm i playwright)
//
// What it looks at, for both languages: console errors, failed requests,
// requests that leave the host, CSP violations, inline style or script,
// lang/hreflang/canonical, head resources, internal links and anchors, images
// (broken, dimensions, alt text, every srcset file), heading order, landmarks,
// the lightbox from the keyboard, tab order, 360/768/1280/1600 px in light and
// dark for horizontal overflow, the page with JavaScript off, and the 404 page.
const { chromium } = require('playwright');
const BASE = process.argv[2] || 'http://127.0.0.1:8198/';
let failures = 0;
const ok = (cond, label, detail = '') => {
  console.log((cond ? 'PASS ' : 'FAIL ') + label + (detail ? ' — ' + detail : ''));
  if (!cond) failures++;
};
const external = new Set();

async function open(browser, path, opts = {}) {
  const ctx = await browser.newContext({
    viewport: { width: opts.width || 1280, height: 900 },
    colorScheme: opts.scheme || 'light',
    javaScriptEnabled: opts.js !== false,
    deviceScaleFactor: opts.dsf || 1,
  });
  const page = await ctx.newPage();
  const log = { requests: [], errors: [], bad: [], csp: [] };
  page.on('request', (r) => log.requests.push(r.url()));
  page.on('console', (m) => {
    if (m.type() === 'error' || m.type() === 'warning') log.errors.push(m.type() + ': ' + m.text());
  });
  page.on('pageerror', (e) => log.errors.push('pageerror: ' + e.message));
  page.on('requestfailed', (r) => log.bad.push('failed ' + r.url()));
  page.on('response', (r) => {
    if (r.status() >= 400) log.bad.push(r.status() + ' ' + r.url());
  });
  if (opts.js !== false)
    await page.addInitScript(() => {
      window.__csp = [];
      document.addEventListener('securitypolicyviolation', (e) =>
        window.__csp.push(e.violatedDirective + ' ' + e.blockedURI),
      );
    });
  await page.goto(BASE + path, { waitUntil: 'load' });
  return { ctx, page, log };
}

async function loadAll(page) {
  await page.evaluate(() => {
    document.querySelectorAll('img[loading=lazy]').forEach((i) => {
      i.loading = 'eager';
    });
  });
  await page.waitForFunction(
    () => [...document.images].filter((i) => i.getAttribute('src')).every((i) => i.complete),
    null,
    { timeout: 15000 },
  );
}

(async () => {
  const browser = await chromium.launch();
  const status = async (ctx, url) => (await ctx.request.get(url)).status();

  for (const path of ['', 'pl/']) {
    const name = path || 'en';
    const { ctx, page, log } = await open(browser, path);
    await loadAll(page);
    await page.waitForTimeout(300);

    ok(log.errors.length === 0, `[${name}] no console errors or warnings`, log.errors.join(' | '));
    ok(log.bad.length === 0, `[${name}] no failed or 4xx/5xx responses`, log.bad.join(' | '));
    const off = log.requests.filter((u) => !u.startsWith(BASE) && !u.startsWith('data:'));
    ok(
      off.length === 0,
      `[${name}] every request stays on localhost (${log.requests.length} requests)`,
      off.join(' | '),
    );
    const csp = await page.evaluate(() => window.__csp);
    ok(csp.length === 0, `[${name}] no Content-Security-Policy violations`, csp.join(' | '));
    ok(
      await page.evaluate(
        () =>
          !document.querySelector('[style]') &&
          !document.querySelector('style') &&
          ![...document.scripts].some((s) => !s.src),
      ),
      `[${name}] no inline style or script in the document`,
    );

    // language
    const lang = await page.evaluate(() => document.documentElement.lang);
    ok(lang === (path ? 'pl' : 'en'), `[${name}] html lang is ${lang}`);
    const alts = await page.evaluate(() =>
      [...document.querySelectorAll('link[rel=alternate][hreflang]')].map((l) => l.hreflang + '=' + l.href),
    );
    ok(
      alts.length === 3 &&
        alts.includes('en=' + BASE) &&
        alts.includes('pl=' + BASE + 'pl/') &&
        alts.includes('x-default=' + BASE),
      `[${name}] hreflang links`,
      alts.join(' '),
    );
    ok(
      await page.evaluate(() => document.querySelector('link[rel=canonical]').href === location.href),
      `[${name}] canonical resolves to the page itself`,
    );

    // head resources
    const head = await page.evaluate(() =>
      [...document.querySelectorAll('link[href]')]
        .map((l) => l.href)
        .concat(
          [...document.querySelectorAll('meta[property="og:image"],meta[name="twitter:image"]')].map(
            (m) => new URL(m.content, location.href).href,
          ),
        ),
    );
    for (const u of new Set(head)) {
      const s = await status(ctx, u);
      ok(s === 200, `[${name}] head resource ${u.replace(BASE, '/')}`, String(s));
    }
    const meta = await page.evaluate(() => ({
      title: document.title,
      desc: document.querySelector('meta[name=description]').content.length,
      og: ['og:title', 'og:description', 'og:image', 'og:type', 'og:locale'].every((p) =>
        document.querySelector(`meta[property="${p}"]`),
      ),
      tw: document.querySelector('meta[name="twitter:card"]').content,
    }));
    ok(
      meta.title.length > 20 &&
        meta.desc > 80 &&
        meta.desc < 200 &&
        meta.og &&
        meta.tw === 'summary_large_image',
      `[${name}] title, description (${meta.desc} chars), Open Graph and Twitter tags`,
    );

    // links and anchors
    const links = await page.evaluate(() =>
      [...document.querySelectorAll('a[href]')].map((a) => ({
        href: a.href,
        raw: a.getAttribute('href'),
        text: (
          a.getAttribute('aria-label') ||
          a.textContent.trim() ||
          (a.querySelector('img') || {}).alt ||
          ''
        )
          .replace(/\s+/g, ' ')
          .trim(),
      })),
    );
    ok(
      links.every((l) => l.text.length > 0),
      `[${name}] every link has a name`,
    );
    const internal = new Set(),
      anchors = new Set();
    for (const l of links) {
      if (l.raw.startsWith('#')) anchors.add(l.raw.slice(1));
      else if (l.href.startsWith(BASE)) internal.add(l.href);
      else external.add(l.href);
    }
    const missing = await page.evaluate(
      (ids) => ids.filter((id) => !document.getElementById(id)),
      [...anchors],
    );
    ok(missing.length === 0, `[${name}] ${anchors.size} in-page anchors resolve`, missing.join(' '));
    for (const u of internal) {
      const s = await status(ctx, u);
      if (s !== 200) ok(false, `[${name}] internal link ${u}`, String(s));
    }
    ok(true, `[${name}] ${internal.size} internal links checked`);

    // images
    const imgs = await page.evaluate(() =>
      [...document.images]
        .filter((i) => i.getAttribute('src'))
        .map((i) => ({
          src: i.currentSrc,
          w: +i.getAttribute('width'),
          h: +i.getAttribute('height'),
          nw: i.naturalWidth,
          nh: i.naturalHeight,
          alt: i.getAttribute('alt'),
          lazy: i.loading,
          dec: i.decoding,
          inFigure: !!i.closest('figure'),
        })),
    );
    ok(
      imgs.every((i) => i.nw > 0),
      `[${name}] ${imgs.length} images, none broken`,
      imgs
        .filter((i) => !i.nw)
        .map((i) => i.src)
        .join(' '),
    );
    ok(
      imgs.every((i) => i.w > 0 && i.h > 0),
      `[${name}] every image has width and height`,
    );
    const skew = imgs.filter((i) => Math.abs(i.w / i.h - i.nw / i.nh) > 0.01);
    ok(
      skew.length === 0,
      `[${name}] width/height match the files' aspect ratio`,
      skew.map((i) => `${i.src.split('/').pop()} ${i.w}x${i.h} vs ${i.nw}x${i.nh}`).join(' | '),
    );
    ok(
      imgs.every((i) => i.alt !== null) && imgs.filter((i) => i.inFigure).every((i) => i.alt.length > 60),
      `[${name}] every screenshot has descriptive alt text`,
    );
    const sets = await page.evaluate(() =>
      [...document.querySelectorAll('img[srcset],source[srcset]')]
        .flatMap((e) => e.srcset.split(',').map((c) => new URL(c.trim().split(/\s+/)[0], location.href).href))
        .concat(
          [...document.querySelectorAll('a.frame')].flatMap((a) =>
            [a.href].concat(a.dataset.dark ? [new URL(a.dataset.dark, location.href).href] : []),
          ),
        ),
    );
    let badset = 0;
    for (const u of new Set(sets)) {
      if ((await status(ctx, u)) !== 200) {
        badset++;
        console.log('  missing ' + u);
      }
    }
    ok(badset === 0, `[${name}] ${new Set(sets).size} srcset / lightbox files exist`);

    // headings
    const heads = await page.evaluate(() =>
      [...document.querySelectorAll('h1,h2,h3,h4')].map((h) => +h.tagName[1]),
    );
    let skip = false;
    for (let i = 1; i < heads.length; i++) if (heads[i] > heads[i - 1] + 1) skip = true;
    ok(
      heads.filter((h) => h === 1).length === 1 && heads[0] === 1 && !skip,
      `[${name}] one h1, no skipped heading level (${heads.length} headings)`,
    );
    ok(
      await page.evaluate(
        () =>
          !!document.querySelector('main') &&
          !!document.querySelector('header') &&
          !!document.querySelector('footer') &&
          !!document.querySelector('nav[aria-label]'),
      ),
      `[${name}] header, nav, main, footer landmarks`,
    );

    // lightbox
    const first = page.locator('.shot a.frame').first();
    await first.scrollIntoViewIfNeeded();
    await first.focus();
    await page.keyboard.press('Enter');
    await page.waitForSelector('dialog.lightbox[open]');
    ok(
      (await page.evaluate(() => {
        const i = document.querySelector('dialog.lightbox img');
        return i.complete && i.naturalWidth > 0 && i.alt.length > 0;
      })) ||
        (await page
          .waitForFunction(() => document.querySelector('dialog.lightbox img').naturalWidth > 0)
          .then(() => true)),
      `[${name}] lightbox opens from the keyboard and shows the picture`,
    );
    ok(
      await page.evaluate(() => document.activeElement.closest('dialog.lightbox') !== null),
      `[${name}] focus moves into the lightbox`,
    );
    const c1 = await page.textContent('dialog.lightbox .count');
    await page.keyboard.press('ArrowRight');
    const c2 = await page.textContent('dialog.lightbox .count');
    ok(c1 !== c2, `[${name}] arrow key moves to the next screenshot`, `${c1} → ${c2}`);
    await page.keyboard.press('ArrowLeft');
    // Tab stays inside the dialog
    for (let i = 0; i < 6; i++) await page.keyboard.press('Tab');
    ok(
      await page.evaluate(
        () =>
          document.activeElement === document.body ||
          document.activeElement.closest('dialog.lightbox') !== null,
      ),
      `[${name}] Tab never reaches the page behind the open lightbox`,
    );
    await page.keyboard.press('Escape');
    await page.waitForFunction(() => !document.querySelector('dialog.lightbox').open);
    ok(
      await page.evaluate(() => document.activeElement === document.querySelector('.shot a.frame')),
      `[${name}] Escape closes the lightbox and returns focus to the link`,
    );
    await first.click();
    await page.waitForSelector('dialog.lightbox[open]');
    await page.click('dialog.lightbox button[aria-label]:last-of-type');
    ok(
      await page.evaluate(() => !document.querySelector('dialog.lightbox').open),
      `[${name}] close button closes the lightbox`,
    );

    await ctx.close();
    // keyboard order from the top, in a fresh page
    const fresh = await open(browser, path);
    const order = [];
    for (let i = 0; i < 16; i++) {
      await fresh.page.keyboard.press('Tab');
      order.push(
        await fresh.page.evaluate(() => {
          const e = document.activeElement;
          const img = e.querySelector && e.querySelector('img');
          return (
            e.getAttribute('aria-label') ||
            e.textContent.trim() ||
            (img ? 'screenshot: ' + img.alt : e.tagName)
          )
            .replace(/\s+/g, ' ')
            .trim()
            .slice(0, 30);
        }),
      );
    }
    console.log(`INFO [${name}] tab order: ` + order.join(' › '));
    const outline = await fresh.page.evaluate(
      () =>
        getComputedStyle(document.activeElement).outlineStyle +
        ' ' +
        getComputedStyle(document.activeElement).outlineWidth,
    );
    ok(/solid 2px/.test(outline), `[${name}] focused element has a visible outline`, outline);
    await fresh.ctx.close();
  }

  // viewport matrix
  for (const path of ['', 'pl/'])
    for (const scheme of ['light', 'dark'])
      for (const width of [360, 768, 1280, 1600]) {
        const { ctx, page, log } = await open(browser, path, { width, scheme });
        await loadAll(page);
        const r = await page.evaluate(() => {
          const vw = document.documentElement.clientWidth;
          const wide = [...document.querySelectorAll('body *')]
            .filter((e) => {
              if (e.closest('pre') || e.closest('dialog')) return false;
              const b = e.getBoundingClientRect();
              return b.width > 0 && (b.right > vw + 1 || b.left < -1);
            })
            .map((e) => e.tagName + '.' + e.className);
          return {
            sw: document.documentElement.scrollWidth,
            vw,
            wide: [...new Set(wide)].slice(0, 5),
            bg: getComputedStyle(document.body).backgroundColor,
            hero: document.querySelector('.hero-shot img').currentSrc.split('/').pop(),
          };
        });
        const expectHero = scheme === 'dark' ? /board-dark/ : /board-light/;
        ok(
          r.sw <= r.vw && r.wide.length === 0 && log.errors.length === 0 && expectHero.test(r.hero),
          `[${path || 'en'} ${width}px ${scheme}] no horizontal overflow, no errors, hero is ${r.hero}`,
          r.wide.join(' ') + ' ' + log.errors.join(' '),
        );
        await ctx.close();
      }

  // dark lightbox picks the dark file
  {
    const { ctx, page } = await open(browser, '', { scheme: 'dark' });
    await page.locator('.shot a.frame').first().click();
    await page.waitForSelector('dialog.lightbox[open]');
    ok(
      /board-dark-1600/.test(await page.getAttribute('dialog.lightbox img', 'src')),
      '[en dark] lightbox shows the dark variant',
    );
    await ctx.close();
  }

  // without JavaScript
  for (const path of ['', 'pl/']) {
    const withJs = await open(browser, path);
    const a = await withJs.page.evaluate(
      () => document.querySelector('main').innerText.replace(/\s+/g, ' ').length,
    );
    await withJs.ctx.close();
    const { ctx, page, log } = await open(browser, path, { js: false });
    const text = await page.locator('main').innerText();
    const b = text.replace(/\s+/g, ' ').length;
    const hidden = await page.locator('main :is(h1,h2,h3,p,li,dd,figcaption,pre)').evaluateAll(
      (els) =>
        els.filter((e) => {
          const s = getComputedStyle(e);
          const r = e.getBoundingClientRect();
          return s.display === 'none' || s.visibility === 'hidden' || +s.opacity === 0 || r.width === 0;
        }).length,
    );
    const dialogs = await page.locator('dialog').count();
    const copy = await page.locator('.copy').count();
    ok(
      Math.abs(a - b) < 40 && hidden === 0 && dialogs === 0 && copy === 0 && log.bad.length === 0,
      `[${path || 'en'} no JS] page reads fully (${b} characters of text vs ${a} with JS, nothing hidden)`,
      `hidden ${hidden}`,
    );
    const href = await page.locator('.shot a.frame').first().getAttribute('href');
    ok(
      (await status(ctx, new URL(href, BASE + path).href)) === 200,
      `[${path || 'en'} no JS] a screenshot link opens the image file`,
    );
    const sw = await page.locator('.site-tools .lang a').getAttribute('href');
    ok(
      (await status(ctx, new URL(sw, BASE + path).href)) === 200,
      `[${path || 'en'} no JS] the other language is one plain link away (${sw})`,
    );
    await ctx.close();
  }

  // 404 page
  {
    const ctx = await browser.newContext();
    const page = await ctx.newPage();
    const errs = [];
    page.on('console', (m) => {
      if (m.type() === 'error' && !/404/.test(m.text())) errs.push(m.text());
    });
    const resp = await page.goto(BASE + 'no/such/page');
    const st = resp.status();
    const h1 = st === 404 ? await page.locator('h1').count() : 0;
    const styled =
      st === 404 && h1
        ? await page.evaluate(() =>
            getComputedStyle(document.querySelector('h1')).fontFamily.includes('Newsreader'),
          )
        : false;
    console.log(
      `INFO 404: status ${st}, custom page ${h1 ? 'yes' : 'no'}, styled ${styled}, errors ${errs.length}`,
    );
    await ctx.close();
  }

  console.log('EXTERNAL ' + [...external].sort().join('\n         '));
  await browser.close();
  console.log(failures ? `\n${failures} FAILURE(S)` : '\nALL CHECKS PASSED');
  process.exit(failures ? 1 : 0);
})();
