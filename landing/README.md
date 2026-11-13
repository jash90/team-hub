# team-hub landing page

The first page a visitor of a team-hub instance sees: what team-hub is, what makes it
different, how to run it, where to get the desktop app, and what it does not do yet, with
the way to the sign-in. English at `/`, Polish at `/pl/`.

It is not a site of its own. The server serves this folder (`crates/hub-server/src/landing.rs`):

- `/` is this page for somebody who is not signed in, and the application for somebody
  who is. An instance nobody has set up yet shows its setup form there instead.
- `/pl/` is the Polish page, for anyone.
- **Sign in** leads to `/login`, which is the application's sign-in form; **Get the desktop
  app** leads to the download chapter and from there to the releases on GitHub.
- `TEAM_HUB_LANDING=off` on the server leaves the page out: a visitor then gets the
  sign-in right away. `TEAM_HUB_LANDING_DIR` says where this folder is (`landing`, and
  `/app/landing` in the image).

Hand-written HTML, one stylesheet and one small script. There is no build step, no
framework and no dependency: what is in this folder is what is served. The page makes no
request to any other host (no analytics, no trackers, no fonts or scripts from elsewhere),
and it reads fully with JavaScript off.

```
index.html                  the English page
pl/index.html               the Polish page
robots.txt
landing-assets/site.css     the only stylesheet, shared by both pages
landing-assets/site.js      the lightbox for screenshots and the copy buttons; an enhancement only
landing-assets/img/         screenshots: WebP, most in a 1600 px and an 800 px width; en-* for
                            the English page, pl-* for the Polish one
landing-assets/fonts/       Newsreader (headings), two subsets, with its licence (OFL.txt)
landing-assets/og-*.png     link-preview images, 1200 x 630
landing-assets/favicon.*    the tracker's own mark
tools/                      two optional scripts, see "Checking it"; nothing here is served
```

The folder with the files is called `landing-assets` and not `assets` because the pages
live at the same address as the application, whose own files are under `/assets`.

## Previewing it

The pages alone, without a server:

```bash
python3 -m http.server 8198
```

in this folder, then <http://localhost:8198/> and <http://localhost:8198/pl/>. The sign-in
link leads nowhere there, and the response headers are not the server's.

As it is served: start team-hub from the top of the repository (`cargo run -p hub-server`,
with the interface built) and open it signed out.

## How it is served

By the team-hub server, from this folder; the image copies the two pages, `robots.txt`
and `landing-assets/` to `/app/landing`. What the server does:

- a Content-Security-Policy on both pages that allows only this origin (`default-src
  'none'`, then `'self'` for images, style, script and fonts);
- `Cache-Control`: `no-store` for `/`, because what is there depends on who asks; five
  minutes for `/pl/`; a year and `immutable` for `landing-assets/img/`,
  `landing-assets/fonts/` and for `site.css` / `site.js` when they are asked for with
  `?v=`; an hour for the other files;
- `/pl` → `/pl/`;
- a file that `landing-assets/` does not have is a 404; any other address is the
  application's.

Because of that policy the pages must not contain inline `<script>`, inline `<style>` or
`style="…"` attributes. `tools/verify.js` fails when one appears.

### When something changes

- **`site.css` or `site.js`**: raise the number in `?v=N` in `index.html` and
  `pl/index.html`. Browsers keep the old file for a year otherwise.
- **A screenshot**: give the new file a new name. Files under `landing-assets/img/` are
  cached for a year under their name.
- **Pages**: nothing to do.

### The site's address

Every address in the pages is relative, except the sign-in (`/login`), so the pages work
on any domain. Search engines and link previews prefer absolute addresses in a few
places. For an instance with a public address (say `https://team-hub.example`), put it in:

1. `<link rel="canonical">` in both pages (`https://team-hub.example/` and `…/pl/`);
2. the three `<link rel="alternate" hreflang>` lines in both pages;
3. `og:url`, `og:image` and `twitter:image` in both pages.

## Where the copy lives

In the two HTML files, nowhere else. They have the same structure, section for section
and figure for figure, so a change in one is easy to find in the other. Change both.

The source of every claim is the repository's own documentation: `README.md` at the top,
`apps/desktop/README.md` and `apps/desktop/RELEASE_NOTES.md`, `docs/architecture.md`,
`docs/agent-runs.md`, `docs/runners.md`, `docs/cloud-runner.md`, `docs/e2e-jobs.md`,
`docs/schedule.md` and the modules' READMEs. The page says nothing they do not say. When
those change, this page should follow, in particular:

- **"Not yet"** (`#not-yet`) mirrors "What it does not do yet" and "Known gaps" in the
  top README. The same admissions are repeated next to the feature they concern (the
  indented notes with a rule on their left): keep the two in step.
- **"Download"** (`#download`) follows `apps/desktop/RELEASE_NOTES.md`: what the first
  start downloads and how big it is, Node.js and npm, unsigned builds, and the table of the
  release's files. The file names carry the version, so the table, its caption and the
  sentence "0.1.0 is the first" change with every release. The button points at
  `/releases/latest` and stays as it is. The wording is true before the release is
  published too ("if the page shows no release yet").
- **"How it is built"** (`#built`): the sign-in row repeats `docs/architecture.md`,
  "Sign-in and safety", and nothing more. The page does not say what the server needs
  "nothing" of: say what is there (one binary, SQLite in one file, runners for work in
  code) instead.
- **The licence** is MIT (`LICENSE` at the top of the repository): named in "How it is
  built" and in the footer.
- **Captures that are not the real thing say so in their caption**: in the desktop thread
  and its push preview a scripted stand-in for the model wrote the agent's lines and the
  commit (the app's core and the server are real); the test session was posted by a script
  that plays the runner, with a recording put together from screenshots; the test job in
  the desktop Tests view was carried out by the same script.

Polish: the words are the interface's own (`zadanie`, `tablica`, `backlog`, `baza wiedzy`,
`wydanie`, `oś czasu`, `przebieg` for a test run, `uruchomienie testów` for a test job,
`sesja` and `nagranie` for what a run opens as, `zlecenie` for an agent run, `propozycja`,
`wątek`, `drzewo robocze`, `runner` and `wspólny runner`, `harmonogram` and, for a
scheduled task, `czynność`, because `zadanie` is taken by issues). Single-letter words (`w`, `z`, `i`, `a`, `o`,
`u`) are followed by `&nbsp;` so that they never end a line; keep that when editing.

The headline and the kicker also appear in the link-preview images; after changing them,
run `tools/og-image.js`.

## Adding a screenshot

Screenshots are real captures of the running product, cropped to the part that matters.
Nothing is mocked up.

**Each page shows its own language, interface and data.** The English page uses only
`en-*` files, captured from an instance set up with the English example data; the Polish
page only `pl-*` files, from one with the Polish data. A capture in the other language is
never used with a caption that explains it: when a state cannot be captured in a language,
the figure is left out of that page. `tools/verify.js` fails when a page references a
screenshot without its prefix or when the two pages share one.

To get the two instances: start the server twice with a data directory each, and set each
up with `POST /api/setup` `{ "name", "email", "password", "demo": true, "demoLanguage":
"en" | "pl" }` (the same projects, keys and statuses in either language). Capture with
the browser in the matching language (`locale: 'en-US'` / `'pl-PL'` in Playwright; for
Polish also start the browser with `LANG=pl_PL.UTF-8`, or a date field shows a US date).
For the English instance set the schedule's time zone to something other than the
default Europe/Warsaw first (`PUT /api/admin/schedule`). The desktop app's interface runs
in a browser against its real core (`apps/desktop/README.md`, "Checking it without a model
or a window") and has its own language switch.

1. Capture at 2x. Crop to the panel you are showing (a full window is illegible once it
   is scaled to the column), then write two widths:

   ```bash
   ffmpeg -i shot.png -vf "crop=2364:648:490:486,scale=1600:-1" -quality 82 landing-assets/img/en-name-1600.webp
   ```

   ```bash
   ffmpeg -i shot.png -vf "crop=2364:648:490:486,scale=800:-1" -quality 82 landing-assets/img/en-name-800.webp
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
   to read (`en-board-phone.webp`, `en-board-phone-dark.webp`). When the files of one `<picture>` differ in shape, each
   `<source>` carries its own `width` and `height`.

   Two more classes: `shot small` for a dialog captured at twice its size, so that it is
   not stretched to the column, and a `<div class="stack">` around two figures that belong
   to one claim.

4. Captions are numbered by the stylesheet ("Fig. 3", "Rys. 3"); do not number by hand.
   The lightbox picks up every `.shot a.frame` by itself.

5. Do it in both pages, with a capture in each page's language (`en-name-…`, `pl-name-…`)
   of the same state, and write the alt text and the caption for what that capture shows.

6. Crop the application's own sidebar out: capture the view or the panel, not the window.
   The sidebar lists the modules of the build the capture was made with, and an older
   capture then contradicts the text beside it.

7. `tools/verify.js` knows how many figures each chapter has (`FIGURES` at its top): change
   the number there with the page.

## Checking it

`tools/verify.js` opens both pages in a real browser (Playwright) and checks: no console
errors, no failed request, no request that leaves the host, no CSP violation, no inline
style or script, `lang` / `hreflang` / canonical, every internal link, anchor, image and
`srcset` file, image dimensions and alt text, heading order, the ten chapters and their
numbers, a caption on every figure, the figures each chapter is expected to have, the
download block (the release's files, all of one version; `/releases/latest`; what the
first start needs), the licence, the lightbox from the keyboard (opens, arrows, Escape,
focus returns), tab order, 360 / 768 / 1280 / 1600 px in light and dark for horizontal
overflow and for pictures whose declared size does not match the file shown, the page
with JavaScript off, the way to the sign-in, that both languages have the same chapters, figures
and anchors, and that each page references only screenshots of its own language.

```bash
node tools/verify.js http://127.0.0.1:8080/
```

against a team-hub server that has been set up, opened signed out: that is the page as it
is served, policy included.

`tools/og-image.js` renders the two link-preview images from the page's own stylesheet
(serve the folder with `python3 -m http.server 8198` for it, not with the team-hub server,
whose policy forbids the card's inline style).

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

Newsreader is licensed under the SIL Open Font License 1.1 (`landing-assets/fonts/OFL.txt`); the
two files are the `latin` and `latin-ext` subsets of the variable font as packaged by
Fontsource.
