# Performance baseline (development machine, not target hardware)

Measured with `app/e2e/perf.mjs` on 2026-10-31:

    E2E_SCRIPT=e2e/perf.mjs PAGEWEAVE_CLI=$PWD/target/debug/pageweave app/e2e/run.sh target/debug/pageweave-app <artifacts>

**These are not budgets and not hardware claims.** The machine is a Linux cloud VM (4 vCPU Intel
Xeon 2.1 GHz, 16 GiB RAM, no GPU), the app is a **debug** build, and the window is on a virtual
display (Xvfb) where WebKitGTK composites in software. Windows with WebView2, a GPU and a release
build will differ substantially. Budgets are set only from measurements on minimum and recommended
Windows hardware (`docs/HARDWARE_SUPPORT.md`); this file gives the first reference point and the
method.

## Startup (article.pdf passed at launch, 3 runs)

| | Run 1 (cold) | Run 2 | Run 3 |
|---|---|---|---|
| UI ready (host log) | 1156 ms | 953 ms | 943 ms |
| First page drawn (host log) | 1832 ms | 1764 ms | 1467 ms |
| First page seen from outside | 1923 ms | 2214 ms | 1587 ms |
| Memory of the app's process tree, idle | 835 MiB | 814 MiB | 817 MiB |

## Viewer: a 300-page document (rl-long.pdf)

| | |
|---|---|
| Open to first page | 1923 ms |
| Memory after opening / after scrolling | 870 / 1019 MiB (web view 626, host 346, network 47) |
| Page pictures kept while scrolling | 5 (pages far from the view are dropped) |
| Scroll at 30 px per frame (about 1800 px/s) | median frame 21 ms, p95 86 ms, 13 of 140 frames over 50 ms |
| Scroll at 120 px per frame (about 6 pages/s) | median frame 42 ms, p95 242 ms, 22 of 48 frames over 50 ms |
| Jump to page 250 until its picture is shown | 599 ms |

What the scroll numbers include, measured by elimination (`requestAnimationFrame` alone runs at
16 ms here):
- Scrolling a plain 200 000 px gradient `div` in the same web view: median 28 ms, p95 33 ms — the
  software compositor alone costs about half the frame budget at 120 px/frame.
- Hiding page pictures or text layers brings the viewer's median down to that of the plain `div`;
  the p95 spikes come from pages arriving (picture decode, text layer) and from the thumbnail
  sidebar (closing it: median 24–26 ms, p95 about 130 ms at 120 px/frame).

Changes made while measuring: every page and every thumbnail re-rendered on each scroll-driven
page change because handlers passed to them changed identity on each parent render (one of them,
the annotation-move handlers, added earlier the same day); handlers are now stable and only pages
with move frames get those handlers. Page and thumbnail pictures decode asynchronously. Neither
change moved these numbers measurably on this machine (re-rendering was not the bottleneck here);
both stay because they remove needless main-thread work.

## Engine through the command line (debug build, one run each)

| Operation | Time |
|---|---|
| Inspect 300 pages | 77 ms |
| Text of 300 pages | 842 ms |
| Search 300 pages for "the" | 477 ms |
| Render one page at 2× | 655 ms |
| Word export, article (3 pages) | 90 ms |
| OCR, 3 scanned pages | 22.5 s |
| Translate one page en→de (3 paragraphs, model load included) | 9.6 s |
| Design Studio rebuild of one page picture | 7.5 s |

## Not measured yet

Release build; Windows (WebView2) on minimum/recommended hardware; GPU compositing; long-session
memory stability (hours); low-memory and slow-disk behaviour; high DPI.
