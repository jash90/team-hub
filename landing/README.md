# team-hub landing page

The public page about team-hub: what it is, what makes it different, how to run it, where
to get the desktop app, and what it does not do yet. English at `/`, Polish at `/pl/`.

Hand-written HTML, one stylesheet and one small script. There is no build step, no
framework and no dependency: what is in this folder is what is served. The page makes no
request to any other host (no analytics, no trackers, no fonts or scripts from elsewhere),
and it reads fully with JavaScript off.

```
index.html        the English page
pl/index.html     the Polish page
404.html          served for any path that does not exist (both languages on one page)
robots.txt
assets/site.css   the only stylesheet, shared by all three pages
assets/site.js    the lightbox for screenshots and the copy buttons; an enhancement only
assets/img/       screenshots: WebP, most in a 1600 px and an 800 px width
assets/fonts/     Newsreader (headings), two subsets, with its licence (OFL.txt)
assets/og-*.png   link-preview images, 1200 x 630
assets/favicon.*  the tracker's own mark
Caddyfile         how the site is served: headers, caching, redirects, the 404 page
Dockerfile        caddy:2-alpine with the files above
tools/            two optional scripts, see "Checking it"; nothing here is served
```

## Previewing it

```bash
python3 -m http.server 8198
```

in this folder, then <http://localhost:8198/> and <http://localhost:8198/pl/>. The 404 page
and the response headers only exist under Caddy:

```bash
SITE_ROOT=. PORT=8198 caddy run --config Caddyfile --adapter caddyfile
```

## How it is deployed

As a container that serves this folder with Caddy on `$PORT` (8080 when unset), plain
HTTP; TLS belongs to whatever stands in front.

```bash
docker build -t team-hub-landing .
```

```bash
docker run --rm -p 8080:8080 team-hub-landing
```

On Railway: a service of this repository with **root directory `landing`**, so that this
folder is the build context and the `Dockerfile` here is the one that is found. Railway
sets `PORT`; the healthcheck path can be `/healthz`. No volume and no variables.

What the `Caddyfile` does:

- gzip and zstd for text;
- a Content-Security-Policy that allows only this origin (`default-src 'none'`, then
  `'self'` for images, style, script and fonts), `X-Content-Type-Options`,
  `Referrer-Policy`, `Permissions-Policy`, `X-Frame-Options`, on error pages too;
- `Cache-Control`: a year and `immutable` for `assets/img/`, `assets/fonts/` and for
  `site.css` / `site.js` when they are asked for with `?v=`; an hour for other assets;
  five minutes for pages;
- `/pl` → `/pl/`, `/index.html` → `/`, `/pl/index.html` → `/pl/`, `/en` → `/`;
- `404.html` with status 404 for anything else.

Because of that policy the pages must not contain inline `<script>`, inline `<style>` or
`style="…"` attributes. `tools/verify.js` fails when one appears.

### When something changes

- **`site.css` or `site.js`**: raise the number in `?v=1` in `index.html`,
  `pl/index.html` and `404.html`. Browsers keep the old file for a year otherwise.
- **A screenshot**: give the new file a new name. Files under `assets/img/` are cached
  for a year under their name.
- **Pages**: nothing to do; they are checked again after five minutes.

### The site's address

Every address in the pages is relative, so the site works on any domain and under
`python3 -m http.server`. Search engines and link previews prefer absolute addresses in a
few places. Once the site has its address (say `https://team-hub.example`), put it in:

1. `<link rel="canonical">` in both pages (`https://team-hub.example/` and `…/pl/`);
2. the three `<link rel="alternate" hreflang>` lines in both pages;
3. `og:url`, `og:image` and `twitter:image` in both pages;
4. `robots.txt`: add `Sitemap: https://team-hub.example/sitemap.xml` if you write one
   (two addresses; not worth it before the site has more pages).

## Where the copy lives

In the two HTML files, nowhere else. They have the same structure, section for section
and figure for figure, so a change in one is easy to find in the other. Change both.

The source of every claim is the repository's own documentation: `README.md` at the top,
`apps/desktop/README.md`, `docs/architecture.md`, `docs/agent-runs.md` and the modules'
READMEs. When those change, this page should follow, in particular:

- **"Not yet"** (`#not-yet`) mirrors "What it does not do yet" and "Known gaps" in the
  top README.
- **"Download"** (`#desktop`) is worded to be true before and after the first release of
  the desktop app. Once installers exist and are signed, rewrite it, and drop the sentence
  about building from source being the only way.
- **Nothing about a licence** is said, because the repository has no licence file. The
  page says "source on GitHub".

Polish: the words are the interface's own (`zadanie`, `tablica`, `backlog`, `baza wiedzy`,
`wydanie`, `oś czasu`, `przebieg` for a test run, `zlecenie` for an agent run,
`propozycja`, `wątek`, `drzewo robocze`). Single-letter words (`w`, `z`, `i`, `a`, `o`,
`u`) are followed by `&nbsp;` so that they never end a line; keep that when editing.

The headline and the kicker also appear in the link-preview images; after changing them,
run `tools/og-image.js`.

## Adding a screenshot

Screenshots are real captures of the running product, cropped to the part that matters.
Nothing is mocked up.

1. Capture at 2x. Crop to the panel you are showing (a full window is illegible once it
   is scaled to the column), then write two widths:

   ```bash
   ffmpeg -i shot.png -vf "crop=2364:648:490:486,scale=1600:-1" -quality 82 assets/img/name-1600.webp
   ```

   ```bash
   ffmpeg -i shot.png -vf "crop=2364:648:490:486,scale=800:-1" -quality 82 assets/img/name-800.webp
   ```

   (`crop=width:height:x:y`, in pixels of the capture. A capture narrower than 1600 px
   keeps its own width in the larger file.)

2. Copy a `<figure class="shot">` block from the page and change the file names, `width`
   and `height` (of the larger file; they prevent layout shift), `sizes` (copy from a
   figure in the same position) and the `alt` text, which should say what the screenshot
   shows to someone who cannot see it. Keep `loading="lazy"` and `decoding="async"` for
   anything below the first screen.

3. Light and dark: when the same view exists in both, wrap the `<img>` in `<picture>`
   with a `<source media="(prefers-color-scheme: dark)">`, and put the dark file in
   `data-dark` on the link so that the lightbox opens the right one (the hero is the
   example). When only a light capture exists, give the `<img>` `class="light-only"`: the
   stylesheet dims it a little on a dark page. The hero has two more sources, for
   phones: one column of the board instead of the whole of it, which would be too small
   to read (`board-*-phone.webp`).

4. Captions are numbered by the stylesheet ("Fig. 3", "Rys. 3"); do not number by hand.
   The lightbox picks up every `.shot a.frame` by itself.

5. Do it in both pages. Where the interface was captured in one language only, say so in
   the caption of the other.

## Checking it

`tools/verify.js` opens both pages in a real browser (Playwright) and checks: no console
errors, no failed request, no request that leaves the host, no CSP violation, no inline
style or script, `lang` / `hreflang` / canonical, every internal link, anchor, image and
`srcset` file, image dimensions and alt text, heading order, the lightbox from the
keyboard (opens, arrows, Escape, focus returns), tab order, 360 / 768 / 1280 / 1600 px in
light and dark for horizontal overflow, the page with JavaScript off, and the 404 page.

```bash
node tools/verify.js http://127.0.0.1:8198/
```

`tools/og-image.js` renders the two link-preview images from the page's own stylesheet
(serve the folder with `python3 -m http.server 8198` for it, not with Caddy).

Both need `npm i playwright` somewhere Node can find it; neither is needed to serve or to
edit the page.

## Design notes

One idea carried through: the page is set like a specification sheet. Claims sit in a
narrow left column and the evidence (a real screenshot, a command, a table) sits to the
right of it. Headings are in Newsreader, text in the system's own sans-serif, labels and
code in its monospace. One accent colour, the application's indigo, for links and the
primary action only. Light and dark follow `prefers-color-scheme`; there is no switch.

Colours are the custom properties at the top of `site.css`. Text colours were checked
against their backgrounds for WCAG AA (4.5 : 1 or better, in both schemes); check again
when changing one.

Newsreader is licensed under the SIL Open Font License 1.1 (`assets/fonts/OFL.txt`); the
two files are the `latin` and `latin-ext` subsets of the variable font as packaged by
Fontsource.
