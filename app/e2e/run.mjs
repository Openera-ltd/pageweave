// Real-application end-to-end tests (QUALITY_GATES Gate 3). Drives the built PageWeave binary through
// tauri-driver with real mouse/keyboard input, the real engine and real files.
//
// Usage: node e2e/run.mjs <app-binary> <artifact-dir>   (run via e2e/run.sh, which starts the display + driver)

import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { inflateRawSync } from "node:zlib";
import { basename, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { Session } from "./webdriver.mjs";

const APP = resolve(process.argv[2] ?? "../target/debug/pageweave-app");
const ART = resolve(process.argv[3] ?? "e2e-artifacts");
const DRIVER = process.env.WEBDRIVER_URL ?? "http://127.0.0.1:4444";
const FIX = resolve(fileURLToPath(new URL("../../fixtures/pdf", import.meta.url)));
const CLI = process.env.PAGEWEAVE_CLI; // optional: independent page count check via the CLI
mkdirSync(ART, { recursive: true });

const results = [];
const work = mkdtempSync(join(tmpdir(), "pw-e2e-"));

function assert(cond, msg) {
  if (!cond) throw new Error(`assertion failed: ${msg}`);
}

let currentTest = "";

/** On failure, keep a screenshot and a DOM summary so the cause can be inspected. */
async function captureFailure(s) {
  const slug = currentTest.replace(/[^a-z0-9]+/gi, "-").toLowerCase();
  try {
    writeFileSync(join(ART, `FAIL-${slug}.png`), await s.screenshot());
    const dom = await s.exec(`return JSON.stringify({
      dialogs: [...document.querySelectorAll('[role=dialog]')].map(d => d.innerText),
      notices: [...document.querySelectorAll('.notice')].map(n => n.innerText),
      menu: document.querySelector('[role=menu]')?.innerText ?? null,
      status: document.querySelector('.statusbar')?.innerText,
      search: document.querySelector('.searchbar')?.innerText ?? null,
      focus: document.activeElement?.outerHTML.slice(0, 160)
    }, null, 1)`);
    writeFileSync(join(ART, `FAIL-${slug}.json`), dom);
  } catch {
    /* best effort */
  }
  try {
    // The host log since this launch started (paths are redacted, no document content).
    const dir = await s.execAsync("const done = arguments[arguments.length - 1]; window.__TAURI_INTERNALS__.invoke('log_dir').then(done, (e) => done('ERR ' + e))");
    const log = join(String(dir), "pageweave.jsonl");
    if (!String(dir).startsWith("ERR") && existsSync(log)) {
      const lines = readFileSync(log, "utf8").trim().split("\n");
      const start = lines.map((l) => l.includes('"event":"startup"')).lastIndexOf(true);
      writeFileSync(join(ART, `FAIL-${slug}.log.jsonl`), lines.slice(Math.max(0, start)).slice(-200).join("\n") + "\n");
    }
  } catch {
    /* best effort */
  }
}

const phase = { create: 0, teardown: 0 };
/** App launches that failed before a session existed and were retried (reported in the summary). */
const launchRetries = [];
/** Consecutive scenarios whose app could not be launched at all; the suite stops after 3. */
let launchFailuresInARow = 0;
class LaunchFailed extends Error {}

/**
 * App arguments as the driver must pass them. Edge WebDriver turns every argument into a --switch
 * and keeps one value per switch name, so on Windows files go as --open-1=<path>, --open-2=<path>, …
 */
function appArgs(args) {
  return process.platform === "win32" ? args.map((a, i) => `--open-${i + 1}=${a}`) : args;
}

/**
 * Start the app and wait until the WebView holds the app's own document. Seen on Windows CI (twice in
 * ~60 launches on slow runners): the driver attaches to a blank document (empty <body>, no Tauri
 * bridge) that never navigates to the app. That is a launch failure, not a scenario failure.
 */
async function launch(args) {
  const s = await Session.create(DRIVER, APP, appArgs(args));
  const deadline = Date.now() + 20000;
  let state = "";
  while (Date.now() < deadline) {
    state = await s
      .exec("return JSON.stringify({ href: location.href, ready: document.readyState, root: !!document.getElementById('root'), bridge: !!window.__TAURI_INTERNALS__ })")
      .catch((e) => `ERR ${String(e?.message ?? e).slice(0, 120)}`);
    if (state.includes('"root":true') && state.includes('"bridge":true')) return s;
    await new Promise((r) => setTimeout(r, 200));
  }
  await s.delete().catch(() => undefined);
  throw new Error(`the app document never loaded: ${state}`);
}

async function withApp(args, fn) {
  const t0 = Date.now();
  let s;
  try {
    s = await launch(args);
  } catch (e) {
    // Seen on CI: GTK occasionally fails to connect to the virtual display at process start, and the
    // driver then waits out its timeout; on Windows the WebView can stay on a blank document. Retry
    // the launch once and record it in the summary; a second failure is real.
    launchRetries.push({ test: currentTest, error: String(e?.message ?? e).slice(0, 300) });
    console.log(`  note: app launch failed (${String(e?.message ?? e).slice(0, 160)}); retrying the launch once`);
    try {
      s = await launch(args);
    } catch (e2) {
      launchFailuresInARow++;
      throw new LaunchFailed(`the app could not be started under WebDriver twice: ${String(e2?.message ?? e2).slice(0, 200)}`);
    }
  }
  launchFailuresInARow = 0;
  phase.create += Date.now() - t0;
  try {
    // PDFs open as tabs; Office files open the convert dialog, pictures Create PDF from images and design projects the Design
    // Studio, over the start screen.
    const pdfArgs = args.filter((a) => !/\.(docx|doc|odt|rtf|txt|xlsx|xls|ods|csv|pptx|ppt|odp|html?|pwdesign|png|jpe?g)$/i.test(a));
    await s.waitFor(pdfArgs.length ? ".tab-label" : ".home", { timeout: 20000 });
    return await fn(s);
  } catch (e) {
    await captureFailure(s);
    throw e;
  } finally {
    const t1 = Date.now();
    await s.delete();
    phase.teardown += Date.now() - t1;
  }
}

// E2E_ONLY=<substring>[|<substring>…] runs only matching scenarios (local iteration; CI always runs everything).
const ONLY = process.env.E2E_ONLY;

/** Thrown by a scenario that cannot run on this platform; reported as SKIP, never as PASS. */
class Skip extends Error {}

async function test(name, fn) {
  if (ONLY && !ONLY.split("|").some((o) => name.includes(o))) return;
  if (launchFailuresInARow >= 3) {
    // Every launch is failing: report the remaining scenarios as not run instead of timing out each.
    results.push({ name, status: "FAIL", error: "not run: the app could not be launched for the previous 3 scenarios" });
    console.log(`FAIL ${name}: not run (app launches are failing)`);
    return;
  }
  currentTest = name;
  phase.create = 0;
  phase.teardown = 0;
  const t0 = Date.now();
  try {
    await fn();
    results.push({ name, status: "PASS", ms: Date.now() - t0, create_ms: phase.create, teardown_ms: phase.teardown });
    console.log(`PASS ${name} (${Date.now() - t0} ms; app start ${phase.create} ms, close ${phase.teardown} ms)`);
  } catch (e) {
    if (e instanceof Skip) {
      results.push({ name, status: "SKIP", reason: e.message });
      console.log(`SKIP ${name}: ${e.message}`);
      return;
    }
    results.push({ name, status: "FAIL", ms: Date.now() - t0, error: String(e?.message ?? e) });
    console.log(`FAIL ${name}: ${e?.stack ?? e}`);
  }
}

const pageLoaded = (s, n = 0) =>
  s.waitUntil(() => s.exec(`const i=document.querySelector('[data-page="${n}"] img'); return !!(i && i.complete && i.naturalWidth>0)`), `page ${n + 1} rendered`);
const statusText = (s) => s.exec("return document.querySelector('.statusbar').innerText");
const currentPage = (s) => s.exec("return document.querySelector('.statusbar .page-input').value");
const rootAttr = (s, a) => s.exec(`return document.documentElement.dataset.${a}`);
const noticeTexts = (s) => s.exec("return [...document.querySelectorAll('.notice')].map(n => n.innerText)");

function pdfPages(path) {
  if (!CLI) return null;
  const out = execFileSync(CLI, ["--json", "inspect", path], { encoding: "utf8" });
  return JSON.parse(out).result.data.page_count;
}

/* ------------------------------------------------------------------ */

await test("open from command line renders first page and thumbnails", async () => {
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    assert((await s.exec("return document.querySelector('.tab-label .name').textContent")) === "five-pages.pdf", "tab name");
    assert((await statusText(s)).includes("of 5"), "status shows page count");
    const thumbs = await s.exec("return document.querySelectorAll('.thumb').length");
    assert(thumbs === 5, `5 thumbnails, got ${thumbs}`);
    const overflow = await s.exec("const v=document.querySelector('.viewer'); return v.scrollWidth - v.clientWidth");
    assert(overflow <= 1, `initial zoom fits width (overflow ${overflow}px)`);
    writeFileSync(join(ART, "viewer-light.png"), await s.screenshot());
  });
});

await test("thumbnail click and Ctrl+End / Ctrl+Home navigate pages", async () => {
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.click('[data-thumb="3"]');
    await s.waitUntil(async () => (await currentPage(s)) === "4", "page 4 after thumbnail click");
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "End"]);
    await s.waitUntil(async () => (await currentPage(s)) === "5", "last page");
    await s.keys(["Control", "Home"]);
    await s.waitUntil(async () => (await currentPage(s)) === "1", "first page");
  });
});

await test("real-world documents (test corpus): each opens, first and last pages render, search answers, no error", async () => {
  // Built by tools/corpus/build.py (public-domain downloads and PDFs from LibreOffice, Chromium,
  // reportlab, cairo and ImageMagick); not committed, so skipped when it has not been built.
  const corpus = resolve(fileURLToPath(new URL("../../target/corpus", import.meta.url)));
  const files = ["real", "made"].flatMap((d) => (existsSync(join(corpus, d)) ? readdirSync(join(corpus, d)).filter((f) => f.endsWith(".pdf")).map((f) => join(corpus, d, f)) : []));
  if (!files.length || !CLI) throw new Skip("test corpus not built (python3 tools/corpus/build.py) or no CLI");
  for (const f of files) {
    const pages = pdfPages(f);
    await withApp([f], async (s) => {
      await s.waitUntil(() => s.exec("return !!document.querySelector('[data-page=\"0\"] img') || !!document.querySelector('.banner.error, [role=alert]')"), `${basename(f)}: first page or an error`, 60000);
      const error = await s.exec("return document.querySelector('.banner.error, [role=alert]')?.innerText ?? null");
      assert(error === null, `${basename(f)}: error shown: ${error}`);
      await pageLoaded(s, 0);
      if (pages > 1) {
        await s.keys(["Control", "g"]);
        await s.type(".statusbar .page-input", String(pages));
        await s.keys(["Enter"]);
        await s.waitUntil(async () => (await currentPage(s)) === String(pages), `${basename(f)}: page ${pages}`, 30000);
        await pageLoaded(s, pages - 1);
      }
      await s.keys(["Control", "f"]);
      await s.type('input[aria-label="Find in document"]', "the");
      await s.keys(["Enter"]);
      await s.waitUntil(async () => /of \d+|No matches/.test(await s.exec("return document.querySelector('.searchbar .count')?.textContent ?? ''")), `${basename(f)}: search answered`, 60000);
      assert(!(await s.exec("return !!document.querySelector('.banner.error')")), `${basename(f)}: no error after viewing`);
    });
  }
});

await test("go to page via status bar input", async () => {
  await withApp([join(FIX, "twenty-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", "g"]);
    await s.type(".statusbar .page-input", "17");
    await s.keys(["Enter"]);
    await s.waitUntil(async () => (await currentPage(s)) === "17", "page 17");
    await pageLoaded(s, 16);
  });
});

await test("zoom buttons, preset select and Ctrl+0", async () => {
  await withApp([join(FIX, "single-page.pdf")], async (s) => {
    await pageLoaded(s, 0);
    const zoom = () => s.exec("return document.querySelector('select[aria-label=\"Zoom level\"]').value");
    await s.keys(["Control", "0"]);
    await s.waitUntil(async () => (await zoom()) === "100", "100% after Ctrl+0");
    await s.click('[data-command="view.zoomIn"]');
    await s.waitUntil(async () => (await zoom()) === "125", "125% after zoom in");
    await s.click('[data-command="view.zoomOut"]');
    await s.click('[data-command="view.zoomOut"]');
    await s.waitUntil(async () => (await zoom()) === "75", "75% after zoom out twice");
    const w = await s.exec("return document.querySelector('[data-page=\"0\"]').getBoundingClientRect().width");
    assert(Math.abs(w - 612 * 0.75 * (96 / 72)) < 2, `page width at 75% (${w})`);
    // Ctrl + wheel zooms smoothly (not only preset steps); the same without Ctrl scrolls instead.
    const wheel = (dy, ctrl) =>
      s.exec(`const v = document.querySelector('[data-testid="viewer"]'); const r = v.getBoundingClientRect();
        v.dispatchEvent(new WheelEvent('wheel', { deltaY: ${dy}, ctrlKey: ${ctrl}, clientX: r.left + 100, clientY: r.top + 100, bubbles: true, cancelable: true }));`);
    await wheel(-120, true);
    await s.waitUntil(async () => Number(await zoom()) > 75, "Ctrl + wheel up zooms in");
    const z1 = Number(await zoom());
    await wheel(240, true);
    await s.waitUntil(async () => Number(await zoom()) < z1, "Ctrl + wheel down zooms out");
    const z2 = Number(await zoom());
    await wheel(120, false);
    await new Promise((r) => setTimeout(r, 300));
    assert(Number(await zoom()) === z2, "plain wheel does not zoom");
  });
});

await test("find: Ctrl+F, Enter, F3 cycle and highlight; Escape closes", async () => {
  await withApp([join(FIX, "search-terms.pdf")], async (s) => {
    const countText = () => s.exec("return document.querySelector('.searchbar .count')?.textContent ?? ''");
    await pageLoaded(s, 0);
    await s.keys(["Control", "f"]);
    await s.waitFor('input[aria-label="Find in document"]');
    await s.type('input[aria-label="Find in document"]', "alpha");
    await s.keys(["Enter"]);
    await s.waitUntil(async () => (await countText()) === "1 of 5", "1 of 5");
    const hl = await s.exec("return document.querySelectorAll('.hit').length");
    assert(hl >= 5, `highlights drawn (${hl})`);
    await s.keys(["F3"]);
    await s.waitUntil(async () => (await countText()) === "2 of 5", "2 of 5");
    await s.keys(["Shift", "F3"]);
    await s.waitUntil(async () => (await countText()) === "1 of 5", "back to 1 of 5");
    // Toggling an option re-runs the search without pressing Enter again.
    await s.click('button[aria-label="Whole words only"]');
    await s.waitUntil(async () => (await countText()) === "1 of 4", "whole word re-search 1 of 4");
    await s.click('button[aria-label="Match case"]');
    await s.waitUntil(async () => (await countText()) === "1 of 2", "case + whole word 1 of 2");
    writeFileSync(join(ART, "search.png"), await s.screenshot());
    await s.keys(["Escape"]);
    await s.waitUntil(async () => (await s.exec("return !document.querySelector('.searchbar')")), "search closed");
    assert((await s.exec("return document.querySelectorAll('.hit').length")) === 0, "highlights cleared");
  });
});

await test("find with no matches says so", async () => {
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await s.keys(["Control", "f"]);
    await s.type('input[aria-label="Find in document"]', "zzzz-not-here");
    await s.keys(["Enter"]);
    await s.waitUntil(async () => (await s.text(".searchbar .count")) === "No matches", "No matches");
  });
});

await test("extract pages: keyboard shortcut, live range validation, output verified", async () => {
  const out = join(work, "extract-out.pdf");
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", "Shift", "E"]);
    await s.waitFor('[role="dialog"]');
    await s.type("#pages", "9");
    await s.waitUntil(async () => (await s.text("#pages-msg")).includes("does not exist"), "out-of-range message");
    assert((await s.attr('[data-testid="tool-start"]', "aria-disabled")) === "true", "start disabled while invalid");
    await s.type("#pages", "2-3");
    await s.waitUntil(async () => (await s.text("#pages-msg")) === "2 pages selected", "valid range");
    await s.type("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved extract-out.pdf")), "success notice");
    assert(existsSync(out), "output exists");
    const n = pdfPages(out);
    assert(n === null || n === 2, `output has 2 pages (got ${n})`);
  });
});

await test("existing output: Keep both creates a renamed copy; original untouched", async () => {
  const out = join(work, "conflict.pdf");
  writeFileSync(out, "existing");
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await s.keys(["Control", "Shift", "E"]);
    await s.type("#pages", "1");
    await s.waitUntil(async () => (await s.text("#pages-msg")) === "1 page selected", "valid");
    await s.type("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitFor(".dialog .banner.error");
    assert((await s.text(".dialog .banner.error")).includes("already exists"), "conflict explained");
    await s.click(".dialog .banner.error .btn.outline");
    await s.waitUntil(async () => existsSync(join(work, "conflict (1).pdf")), "renamed output");
  });
  const { readFileSync } = await import("node:fs");
  assert(readFileSync(out, "utf8") === "existing", "existing file not overwritten");
});

await test("rotate a page from the keyboard context menu (Shift+F10)", async () => {
  const out = join(work, "rotated.pdf");
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.click('[data-testid="viewer"]');
    await s.keys(["Shift", "F10"]);
    await s.waitFor('[role="menu"]');
    const focused = await s.exec("return document.activeElement?.getAttribute('role')");
    assert(focused === "menuitem", `first menu item focused (${focused})`);
    await s.keys(["ArrowDown"]);
    await s.keys(["Enter"]);
    await s.waitFor('[role="dialog"]');
    assert((await s.exec("return document.querySelector('#pages').value")) === "1", "range prefilled with page 1");
    await s.waitUntil(async () => (await s.text("#pages-msg")) === "1 page selected", "valid");
    await s.type("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => existsSync(out), "rotated output");
  });
});

await test("right-click page menu lists page actions; Escape closes it", async () => {
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.contextClick('[data-page="0"]');
    await s.waitFor('[role="menu"]');
    const labels = await s.exec("return [...document.querySelectorAll('[role=menuitem]')].map(m => m.innerText.trim())");
    for (const want of ["Extract page 1…", "Rotate page 1…", "Delete page 1…", "Zoom in", "Document properties"]) {
      assert(labels.some((l) => l.startsWith(want)), `menu has ${want}: ${labels}`);
    }
    const inside = await s.exec("const r=document.querySelector('[role=menu]').getBoundingClientRect(); return r.right<=innerWidth && r.bottom<=innerHeight && r.left>=0 && r.top>=0");
    assert(inside, "menu inside the window");
    writeFileSync(join(ART, "context-menu.png"), await s.screenshot());
    await s.keys(["Escape"]);
    await s.waitUntil(async () => await s.exec("return !document.querySelector('[role=menu]')"), "menu closed");
  });
});

await test("delete refuses removing every page", async () => {
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await s.keys(["Control", "Shift", "D"]);
    await s.type("#pages", "1-5");
    await s.waitUntil(async () => (await s.attr('[data-testid="tool-start"]', "title")) === "A PDF must keep at least one page.", "blocked with reason");
  });
});

await test("settings: live theme preview, Cancel restores, Save persists across restart", async () => {
  await withApp([join(FIX, "single-page.pdf")], async (s) => {
    const before = await rootAttr(s, "theme");
    await s.keys(["Control", ","]);
    await s.waitFor('[role="dialog"]');
    await s.click('[role="radiogroup"][aria-labelledby="seg-theme"] [role="radio"]:nth-child(3)');
    await s.waitUntil(async () => (await rootAttr(s, "theme")) === "dark", "dark preview");
    writeFileSync(join(ART, "settings-dark.png"), await s.screenshot());
    await s.keys(["Escape"]);
    await s.waitUntil(async () => (await rootAttr(s, "theme")) === before, "cancel restored theme");
    await s.keys(["Control", ","]);
    await s.click('[role="radiogroup"][aria-labelledby="seg-theme"] [role="radio"]:nth-child(3)');
    await s.click('[role="radiogroup"][aria-labelledby="seg-density"] [role="radio"]:nth-child(2)');
    await s.click(".dialog .dialog-footer .btn.primary");
    await s.waitUntil(async () => await s.exec("return !document.querySelector('[role=dialog]')"), "dialog closed after save");
  });
  await withApp([join(FIX, "single-page.pdf")], async (s) => {
    await s.waitUntil(async () => (await rootAttr(s, "theme")) === "dark", "dark after restart");
    assert((await rootAttr(s, "density")) === "comfortable", "density persisted");
    await pageLoaded(s, 0);
    writeFileSync(join(ART, "viewer-dark-comfortable.png"), await s.screenshot());
  });
});

await test("custom shortcut: conflicting binding is blocked, unique binding works", async () => {
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await s.keys(["Control", ","]);
    await s.click('button[role="tab"]#tab-shortcuts');
    await s.click('[data-shortcut-row="view.toggleThumbnails"] td:nth-child(3) button:first-child');
    await s.keys(["Control", "o"]);
    await s.waitFor('.dialog .banner.error[role="alert"]');
    assert((await s.attr(".dialog .dialog-footer .btn.primary", "aria-disabled")) === "true", "save blocked by conflict");
    await s.click('[data-shortcut-row="view.toggleThumbnails"] td:nth-child(3) button:first-child');
    await s.keys(["Control", "Alt", "t"]);
    await s.waitUntil(async () => await s.exec("return !document.querySelector('.dialog .banner.error')"), "conflict resolved");
    await s.click(".dialog .dialog-footer .btn.primary");
    await s.waitUntil(async () => await s.exec("return !document.querySelector('[role=dialog]')"), "saved");
    assert(await s.exec("return !!document.querySelector('.sidebar')"), "thumbnails visible");
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "Alt", "t"]);
    await s.waitUntil(async () => await s.exec("return !document.querySelector('.sidebar')"), "new shortcut toggled thumbnails");
  });
});

await test("password-protected PDF: dialog, wrong password, then correct password", async () => {
  const s = await Session.create(DRIVER, APP, appArgs([join(FIX, "encrypted-rc4-user-password.pdf")]));
  try {
    await s.waitFor("#pdf-password", { timeout: 20000 });
    await s.type("#pdf-password", "wrong-password");
    await s.keys(["Enter"]);
    await s.waitUntil(async () => (await s.text("#pw-err")).includes("did not work"), "wrong password message");
    await s.type("#pdf-password", "pw-user-test");
    await s.keys(["Enter"]);
    await s.waitFor('.tab-label');
    await pageLoaded(s, 0);
    assert((await statusText(s)).includes("Protected"), "protected indicator");
    const html = await s.exec("return document.documentElement.outerHTML");
    assert(!html.includes("pw-user-test"), "password not left in the DOM");
  } finally {
    await s.delete();
  }
});

await test("unsupported file shows a specific error and app stays usable", async () => {
  const fake = join(work, "fake.pdf");
  copyFileSync(join(FIX, "not-a-pdf.pdf"), fake);
  const s = await Session.create(DRIVER, APP, appArgs([fake, join(FIX, "single-page.pdf")]));
  try {
    await s.waitFor('.tab-label', { timeout: 20000 });
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Couldn’t open fake.pdf") && t.includes("not a PDF")), "error notice");
    const tabs = await s.exec("return document.querySelectorAll('.tab-label').length");
    assert(tabs === 1, `only the valid file opened (${tabs})`);
    writeFileSync(join(ART, "error-notice.png"), await s.screenshot());
  } finally {
    await s.delete();
  }
});

await test("disabled controls explain why (no document open)", async () => {
  await withApp([], async (s) => {
    // The zoom bar appears with a document; Find is always in the toolbar.
    const title = await s.attr('[data-command="search.open"]', "title");
    assert((await s.attr('[data-command="search.open"]', "aria-disabled")) === "true", "find disabled");
    assert(title.includes("Open a PDF first"), `reason in tooltip: ${title}`);
    const looks = await s.exec(`return [...document.querySelectorAll('[aria-disabled="true"], select:disabled')].map(e => getComputedStyle(e).opacity)`);
    assert(looks.length > 0 && looks.every((o) => Number(o) < 0.6), `disabled controls look disabled (opacities ${looks})`);
    writeFileSync(join(ART, "home.png"), await s.screenshot());
  });
});

await test("command palette runs a command by keyboard", async () => {
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await s.keys(["Control", "k"]);
    await s.type('input[aria-label="Type a command"]', "properties");
    await s.keys(["Enter"]);
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[role=dialog] h2')?.textContent")) === "Document properties", "properties dialog");
    const txt = await s.exec("return document.querySelector('[role=dialog]').innerText");
    assert(txt.includes("Pages") && txt.includes("5"), "properties show page count");
  });
});

await test("tabs: two documents, Ctrl+Tab switches, Ctrl+W closes", async () => {
  await withApp([join(FIX, "single-page.pdf"), join(FIX, "five-pages.pdf")], async (s) => {
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('.tab-label').length")) === 2, "two tabs");
    const active = () => s.exec("return document.querySelector('.tab-label[aria-current=page] .name').textContent");
    const first = await active();
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "Tab"]);
    await s.waitUntil(async () => (await active()) !== first, "switched tab");
    await s.keys(["Control", "w"]);
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('.tab-label').length")) === 1, "one tab left");
  });
});

await test("narrow window keeps toolbar and dialogs usable", async () => {
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await s.setWindowRect(760, 520).catch(() => undefined);
    await s.keys(["Control", "Shift", "M"]);
    await s.waitFor('[role="dialog"]');
    const fits = await s.exec("const r=document.querySelector('[role=dialog]').getBoundingClientRect(); return r.left>=0 && r.top>=0 && r.right<=innerWidth && r.bottom<=innerHeight");
    assert(fits, "dialog inside the window");
    writeFileSync(join(ART, "narrow-merge.png"), await s.screenshot());
  });
});

await test("Ctrl+PageDown / Ctrl+PageUp step pages; previous tab with Ctrl+Shift+Tab", async () => {
  await withApp([join(FIX, "single-page.pdf"), join(FIX, "five-pages.pdf")], async (s) => {
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('.tab-label').length")) === 2, "two tabs");
    await pageLoaded(s, 0);
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "PageDown"]);
    await s.waitUntil(async () => (await currentPage(s)) === "2", "next page");
    await s.keys(["Control", "PageDown"]);
    await s.waitUntil(async () => (await currentPage(s)) === "3", "page 3");
    await s.keys(["Control", "PageUp"]);
    await s.waitUntil(async () => (await currentPage(s)) === "2", "previous page");
    const active = () => s.exec("return document.querySelector('.tab-label[aria-current=page] .name').textContent");
    await s.keys(["Control", "Shift", "Tab"]);
    await s.waitUntil(async () => (await active()) === "single-page.pdf", "previous tab");
    const nextTitle = await s.attr('[data-command="view.zoomIn"]', "title");
    assert(!nextTitle.includes("Open a PDF"), "commands enabled with a document");
  });
});

await test("fit width button restores a fitting zoom; close-find button works", async () => {
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    for (let i = 0; i < 4; i++) await s.click('[data-command="view.zoomIn"]');
    const overflow = () => s.exec("const v=document.querySelector('.viewer'); return v.scrollWidth - v.clientWidth");
    assert((await overflow()) > 50, "zoomed in beyond the window");
    await s.click('[data-command="view.fitWidth"]');
    await s.waitUntil(async () => (await overflow()) <= 1, "fits after Fit width");
    await s.keys(["Control", "f"]);
    await s.waitFor('[data-command="search.close"]');
    await s.click('[data-command="search.close"]');
    await s.waitUntil(async () => await s.exec("return !document.querySelector('.searchbar')"), "find closed by button");
  });
});

await test("Ctrl+D shows document properties", async () => {
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "d"]);
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[role=dialog] h2')?.textContent")) === "Document properties", "properties");
    const t = await s.exec("return document.querySelector('[role=dialog]').innerText");
    assert(t.includes("Five pages") && t.includes("1.7"), `title and version shown: ${t}`);
  });
});

await test("rotate all pages 180° (Ctrl+Shift+R) — rotation verified in the output", async () => {
  const out = join(work, "rotate-all.pdf");
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "Shift", "R"]);
    await s.waitFor('[role="dialog"]');
    assert(await s.exec("return document.querySelector('input[type=checkbox]').checked"), "All pages preselected");
    await s.click('[role="radiogroup"][aria-labelledby="rot-label"] [role="radio"]:nth-child(2)');
    await s.type("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => existsSync(out), "output written");
  });
  if (CLI) {
    const info = JSON.parse(execFileSync(CLI, ["--json", "inspect", out], { encoding: "utf8" })).result.data;
    assert(info.pages.every((p) => p.rotation === 180), `all pages at 180°: ${info.pages.map((p) => p.rotation)}`);
  }
});

await test("keyboard shortcuts settings open from the command palette", async () => {
  await withApp([join(FIX, "single-page.pdf")], async (s) => {
    await s.keys(["Control", "k"]);
    await s.waitFor('input[aria-label="Type a command"]');
    await s.click('[role="option"][data-command="app.shortcuts"]');
    await s.waitUntil(async () => (await s.exec("return document.querySelector('#tab-shortcuts')?.getAttribute('aria-selected')")) === "true", "shortcuts tab selected");
    const rows = await s.exec("return document.querySelectorAll('[data-shortcut-row]').length");
    assert(rows >= 20, `shortcut table lists commands (${rows})`);
  });
});

await test("show-in-folder and log-folder never fail silently", async () => {
  await withApp([join(FIX, "single-page.pdf")], async (s) => {
    for (const id of ["file.reveal", "app.openLogs"]) {
      // ids exercised: "file.reveal", "app.openLogs"
      const before = (await noticeTexts(s)).length;
      await s.keys(["Control", "k"]);
      await s.waitFor('input[aria-label="Type a command"]');
      await s.click(`[role="option"][data-command="${id}"]`);
      // Either the system file manager opens (no notice) or a specific error is shown; the app stays responsive.
      await new Promise((r) => setTimeout(r, 1500));
      const after = await noticeTexts(s);
      if (after.length > before) assert(after.some((t) => t.startsWith("Couldn’t")), `failure explained: ${after}`);
      assert((await s.exec("return document.querySelectorAll('.tab-label').length")) === 1, "app still responsive");
    }
  });
});

const selection = (s) => s.exec("return window.getSelection().toString()");
/** Span in a page's text layer whose text contains `needle`, as a CSS selector via a marker attribute. */
async function markSpan(s, page, needle) {
  await s.waitUntil(() => s.exec(`return !!document.querySelector('[data-textlayer="${page}"] span')`), `text layer on page ${page + 1}`);
  const width = await s.exec(
    `const sp=[...document.querySelectorAll('[data-textlayer="${page}"] span')].find(x=>x.textContent.includes(arguments[0])); if(!sp) return 0; sp.setAttribute('data-e2e','target'); sp.scrollIntoView({block:'center', inline:'center'}); const r=sp.getBoundingClientRect(); return Math.max(r.width, r.height);`,
    needle,
  );
  assert(width > 0, `text run containing ${needle} on page ${page + 1}`);
  return width;
}

await test("double-click selects the word under the pointer; right-click → Copy", async () => {
  await withApp([join(FIX, "single-page.pdf")], async (s) => {
    await pageLoaded(s, 0);
    const w = await markSpan(s, 0, "PWMARK001");
    // "Marker token PWMARK001": aim at ~80% of the run, inside the last word.
    await s.doubleClick('[data-e2e="target"]', w * 0.3, 0);
    await s.waitUntil(async () => (await selection(s)).trim() === "PWMARK001", "word selected");
    // Right-click on the selected word itself: the selection must survive and offer Copy.
    await s.contextClick('[data-e2e="target"]', w * 0.3, 0);
    await s.waitFor('[role="menuitem"][data-command="edit.copy"]');
    await s.click('[role="menuitem"][data-command="edit.copy"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.startsWith("Copied") && t.includes("9 characters")), "copied the 9-character word");
    assert((await selection(s)).trim() === "PWMARK001", "selection still present after using the menu");
    const clip = await s.exec("return navigator.clipboard.readText().then(t => t, e => 'ERR ' + e.name)").catch((e) => `ERR ${e.message}`);
    if (!String(clip).startsWith("ERR")) assert(clip.trim() === "PWMARK001", `clipboard holds the selection (${clip})`);
    else console.log(`  note: clipboard read-back unavailable in this WebDriver session (${clip}); copy success notice verified instead`);
  });
});

await test("text layer is aligned on a 90°-rotated page", async () => {
  await withApp([join(FIX, "mixed-sizes-rotated.pdf")], async (s) => {
    // Regression: the rotated page is laid out landscape and initial fit-width accounts for it.
    const box = await s.exec("const p=document.querySelector('[data-page=\"2\"]'); return [p.offsetWidth, p.offsetHeight]");
    assert(box[0] > box[1], `rotated page laid out landscape (${box})`);
    const overflow = await s.exec("const v=document.querySelector('.viewer'); return v.scrollWidth - v.clientWidth");
    assert(overflow <= 1, `fit width includes the rotated page (overflow ${overflow})`);
    await s.click('[data-thumb="2"]');
    await pageLoaded(s, 2);
    await markSpan(s, 2, "PWMARK003");
    // On a page rotated 90° clockwise the run reads top-to-bottom: aim 80% down its drawn box.
    const r = await s.exec("const r=document.querySelector('[data-e2e=target]').getBoundingClientRect(); return [r.left, r.top, r.width, r.height]");
    assert(r[3] > r[2], `rotated run is taller than wide (${r})`);
    // Regression: the run must not be stretched beyond the rotated page box.
    const page = await s.exec("const p=document.querySelector('[data-page=\"2\"]').getBoundingClientRect(); return [p.top, p.height]");
    assert(r[3] < page[1] * 0.5, `run height ${r[3]} is plausible for page height ${page[1]}`);
    await s.doubleClickAt(r[0] + r[2] / 2, r[1] + r[3] * 0.8);
    await s.waitUntil(async () => (await selection(s)).trim() === "PWMARK003", "word selected on rotated page");
    writeFileSync(join(ART, "rotated-selection.png"), await s.screenshot());
  });
});

// Accessibility (Gate 3): axe-core is injected only into the test session; it is not part of the app.
const AXE = (await import("node:fs")).readFileSync(new URL("../node_modules/axe-core/axe.min.js", import.meta.url), "utf8");
async function axeCheck(s, name) {
  // Measure the settled UI: colours sampled mid-fade (dialogs animate opacity) are not what users see.
  // Two consecutive checks, each after two animation frames, so an animation that has not started
  // yet (pending) on a just-mounted element is not missed.
  const settled = () =>
    s.execAsync("const done = arguments[arguments.length - 1]; requestAnimationFrame(() => requestAnimationFrame(() => done(document.getAnimations().every(a => !a.pending && a.playState !== 'running'))))");
  await s.waitUntil(async () => (await settled()) && (await settled()), "animations settled");
  await s.exec(`${AXE}; return typeof axe`);
  const res = await s.execAsync(
    "const done = arguments[arguments.length - 1]; axe.run(document, { resultTypes: ['violations'] }).then(r => done(r.violations.map(v => ({ id: v.id, impact: v.impact, help: v.help, nodes: v.nodes.length, sample: v.nodes[0]?.target?.join(' '), detail: v.nodes[0]?.any?.[0]?.message })))).catch(e => done([{ id: 'axe-error', impact: 'critical', help: String(e) }]));",
  );
  writeFileSync(join(ART, `axe-${name}.json`), JSON.stringify(res, null, 2));
  const blocking = res.filter((v) => v.impact === "serious" || v.impact === "critical");
  assert(blocking.length === 0, `${name}: ${blocking.map((v) => `${v.id} (${v.impact}, ${v.nodes} nodes, e.g. ${v.sample}): ${v.help}`).join("; ")}`);
  return res;
}

await test("accessibility: no serious/critical axe violations on home, viewer, dialogs (light and dark)", async () => {
  await withApp([], async (s) => {
    await axeCheck(s, "home");
  });
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    for (const theme of ["light", "dark"]) {
      await s.exec(`document.documentElement.dataset.theme = "${theme}"; return 1`);
      await axeCheck(s, `viewer-${theme}`);
      await s.keys(["Control", "f"]);
      await s.type('input[aria-label="Find in document"]', "PWMARK");
      await s.keys(["Enter"]);
      await s.waitUntil(async () => (await s.exec("return document.querySelector('.searchbar .count')?.textContent")) === "1 of 5", "search done");
      await axeCheck(s, `search-${theme}`);
      await s.keys(["Escape"]);
      await s.click('[data-testid="viewer"]');
      await s.keys(["Control", "Shift", "E"]);
      await s.waitFor('[role="dialog"]');
      await axeCheck(s, `extract-dialog-${theme}`);
      await s.keys(["Escape"]);
      await s.keys(["Control", ","]);
      await s.waitFor('[role="dialog"]');
      await axeCheck(s, `settings-${theme}`);
      await s.keys(["Escape"]);
    }
  });
});

await test("bookmarks panel navigates; disabled with a reason when a document has none", async () => {
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "Shift", "B"]);
    await s.waitFor('[data-bookmark="3"]');
    const titles = await s.exec("return [...document.querySelectorAll('.bookmark .title')].map(t => t.textContent)");
    assert(titles.join("|") === "Chapter 1|Chapter 2|Chapter 3|Chapter 4|Chapter 5", `titles ${titles}`);
    await s.click('[data-bookmark="3"]');
    await s.waitUntil(async () => (await currentPage(s)) === "4", "jumped to page 4");
    await s.waitUntil(async () => (await s.attr('[data-bookmark="3"]', "aria-current")) === "location", "current bookmark marked");
    await s.click("#sb-tab-pages");
    await s.waitFor('[data-thumb="0"]');
    await axeCheck(s, "bookmarks-sidebar");
  });
  await withApp([join(FIX, "single-page.pdf")], async (s) => {
    assert((await s.attr("#sb-tab-bookmarks", "aria-disabled")) === "true", "bookmarks tab disabled");
    assert((await s.attr("#sb-tab-bookmarks", "title")).includes("no bookmarks"), "reason shown");
    const cmdTitle = await s.exec("return document.querySelector('#sb-tab-bookmarks').title");
    assert(cmdTitle.length > 0, "tooltip present");
  });
});

const annotCount = (s, kind) => s.exec(`return document.querySelectorAll('.annot${kind ? "." + kind : ""}').length`);
const unsavedText = (s) => s.exec("return document.querySelector('[data-testid=unsaved]')?.textContent ?? ''");
const sha = (p) => createHash("sha256").update(readFileSync(p)).digest("hex");

await test("toolbar: grouped tools, dropdowns, selection tools enable live, overflow in a narrow window", async () => {
  await withApp([join(FIX, "single-page.pdf")], async (s) => {
    // A known width: earlier scenarios in the suite can leave another size. Wait until it has been
    // applied (menus close when the window resizes, so a late resize would race the checks below).
    // The window's resize event can arrive up to ~1 s after the WebDriver call returns (seen on
    // Linux), and menus close on resize: wait for the event itself, then for quiet.
    await s.exec("window.__resizes = 0; window.__resized = 0; addEventListener('resize', () => { window.__resizes++; window.__resized = Date.now(); })");
    const already = (await s.exec("return innerWidth")) === 1366;
    await s.setWindowRect(1366, 860);
    if (!already) await s.waitUntil(async () => await s.exec("return window.__resizes > 0 && Date.now() - window.__resized > 400"), "window resized and settled");
    await pageLoaded(s, 0);
    const groups = await s.exec("return [...document.querySelectorAll('.toolbar [role=group]')].map(g => g.getAttribute('aria-label'))");
    for (const g of ["File", "Comment", "Edit", "Pages", "Convert and read"]) assert(groups.includes(g), `toolbar group ${g} (${groups})`);
    const disabled = (c) => s.exec(`return document.querySelector('.toolbar [data-command="${c}"]').getAttribute('aria-disabled')`);
    // Convert dropdown: opens at its button, lists conversions, Escape closes and restores focus.
    await s.click('[data-testid="menu-convert"]');
    await s.waitFor('[role="menu"][aria-label="Convert"] [data-command="file.toWord"]');
    assert((await s.attr('[data-testid="menu-convert"]', "aria-expanded")) === "true", "dropdown marked open");
    await s.keys(["Escape"]);
    await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('[role=menu]')")), "menu closed");
    assert((await s.exec("return document.activeElement?.dataset.testid")) === "menu-convert", "focus back on the dropdown");
    // Pages dropdown runs a command.
    await s.click('[data-testid="menu-pages"]');
    await s.click('[role="menuitem"][data-command="pages.organize"]');
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[role=dialog] h2')?.textContent")) === "Organize pages", "organize dialog");
    await s.keys(["Escape"]);
    await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('[role=dialog]')")), "dialog closed");
    // Selection tools enable as soon as text is selected, without another click, and work.
    assert((await disabled("annotate.highlight")) === "true", "highlight disabled without a selection");
    const w = await markSpan(s, 0, "PWMARK001");
    await s.doubleClick('[data-e2e="target"]', w * 0.3, 0);
    await s.waitUntil(async () => (await disabled("annotate.highlight")) === null, "highlight enabled by the selection");
    await s.click('.toolbar [data-command="annotate.highlight"]');
    await s.waitUntil(async () => (await annotCount(s, "highlight")) === 1, "highlight drawn from the toolbar");
    await s.waitFor(".tab.selected .dirty");
    await s.click('.toolbar [data-command="edit.undo"]');
    await s.waitUntil(async () => (await annotCount(s, "highlight")) === 0, "undone from the toolbar");
    assert(!(await s.exec("return !!document.querySelector('.tab .dirty')")), "no unsaved dot after undo");
    // At the default (compact) density everything fits at 1366 px; an earlier scenario may have left
    // the comfortable density, where groups may move to More tools (checked below in a narrow window).
    if ((await rootAttr(s, "density")) !== "comfortable") {
      assert(!(await s.exec("return !!document.querySelector('[data-testid=toolbar-more]')")), "everything fits at 1366 px");
    }
    // Narrow: groups move into More tools; nothing overflows the window; their commands still run.
    await s.exec("window.__resizes = 0");
    await s.setWindowRect(760, 520);
    await s.waitUntil(async () => await s.exec("return innerWidth <= 760 && window.__resizes > 0 && Date.now() - window.__resized > 400"), "window narrowed and settled");
    await s.waitFor('[data-testid="toolbar-more"]');
    const fits = await s.exec("const t=document.querySelector('.toolbar'); return t.scrollWidth <= t.clientWidth && [...t.querySelectorAll('button')].every(b => b.getBoundingClientRect().right <= innerWidth)");
    assert(fits, "toolbar fits the narrow window");
    writeFileSync(join(ART, "toolbar-narrow.png"), await s.screenshot());
    await s.click('[data-testid="toolbar-more"]');
    await s.click('[role="menu"][aria-label="More tools"] [data-command="file.protect"]');
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[role=dialog] h2')?.textContent ?? ''")).includes("Protect"), "protect dialog from More tools");
    await s.keys(["Escape"]);
    await s.setWindowRect(1366, 860);
    if ((await rootAttr(s, "density")) !== "comfortable") {
      await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('[data-testid=toolbar-more]')")), "More tools gone when wide again");
    } else {
      await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('.toolbar [data-group]').length")) > 4, "groups come back when wide again");
    }
  });
});

await test("annotate: highlight/underline/strike/note from selection and menus; undo/redo; save a verified copy", async () => {
  const src = join(FIX, "single-page.pdf");
  const before = sha(src);
  const out = join(work, "annotated.pdf");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    const w = await markSpan(s, 0, "PWMARK001");
    const selectWord = async () => {
      await s.doubleClick('[data-e2e="target"]', w * 0.3, 0);
      await s.waitUntil(async () => (await selection(s)).trim() === "PWMARK001", "word selected");
    };
    // Disabled with a reason while nothing is selected.
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Highlight selected");
    const opt = await s.exec("const o=document.querySelector('[role=option][data-command=\"annotate.highlight\"]'); return o ? o.getAttribute('aria-disabled') + '|' + o.textContent : null");
    assert(opt && opt.startsWith("true") && opt.includes("Select text"), `highlight unavailable without a selection (${opt})`);
    await s.keys(["Escape"]);

    // 1. Right-click the selection → Highlight.
    await selectWord();
    await s.contextClick('[data-e2e="target"]', w * 0.3, 0);
    await s.click('[role="menuitem"][data-command="annotate.highlight"]');
    await s.waitUntil(async () => (await annotCount(s, "highlight")) === 1, "highlight drawn");
    assert((await selection(s)) === "", "selection cleared so the highlight is visible");
    await s.waitUntil(async () => (await unsavedText(s)) === "1 unsaved change", "status bar counts it");
    // The overlay covers the word it was made from.
    const cover = await s.exec(`const a=document.querySelector('.annot.highlight').getBoundingClientRect(); const t=document.querySelector('[data-e2e=target]').getBoundingClientRect();
      return [a.left, a.right, a.top, a.bottom, t.left, t.right, t.top, t.bottom]`);
    assert(cover[0] >= cover[4] - 2 && cover[1] <= cover[5] + 2 && Math.abs(cover[2] - cover[6]) < 3 && Math.abs(cover[3] - cover[7]) < 3, `highlight lies on the word (${cover})`);

    // 2. Keyboard shortcut → Underline. 3. Menu → Strikethrough.
    await selectWord();
    await s.keys(["Control", "Shift", "U"]);
    await s.waitUntil(async () => (await annotCount(s, "underline")) === 1, "underline drawn");
    await selectWord();
    await s.contextClick('[data-e2e="target"]', w * 0.3, 0);
    await s.click('[role="menuitem"][data-command="annotate.strikeOut"]');
    await s.waitUntil(async () => (await annotCount(s, "strike_out")) === 1, "strikethrough drawn");

    // Undo / redo (Ctrl+Z, Ctrl+Y, Ctrl+Shift+Z).
    await s.keys(["Control", "z"]);
    await s.waitUntil(async () => (await annotCount(s, "strike_out")) === 0, "undo removed the last annotation");
    await s.keys(["Control", "y"]);
    await s.waitUntil(async () => (await annotCount(s, "strike_out")) === 1, "redo restored it");
    await s.keys(["Control", "z"]);
    await s.keys(["Control", "Shift", "Z"]);
    await s.waitUntil(async () => (await annotCount(s, "strike_out")) === 1, "Ctrl+Shift+Z redoes too");

    // (Add is disabled until text is written.)
    // 4. Right-click blank page area just below the marked line → Add note here…
    await s.contextClick('[data-e2e="target"]', 0, 48);
    await s.click('[role="menuitem"][data-command="annotate.note"]');
    await s.waitFor("#note-text");
    assert((await s.attr('[data-testid="note-add"]', "aria-disabled")) === "true", "Add disabled while the note is empty");
    await s.click('[data-testid="note-add"]');
    assert((await s.exec("return !!document.querySelector('#note-text')")) === true, "empty note is not added");
    await s.type("#note-text", "PW note 1");
    await axeCheck(s, "note-dialog");
    await s.keys(["Control", "Enter"]);
    await s.waitUntil(async () => (await annotCount(s, "note")) === 1, "note marker drawn");
    await s.waitUntil(async () => (await unsavedText(s)) === "4 unsaved changes", "four unsaved changes");
    await axeCheck(s, "viewer-with-annotations");
    writeFileSync(join(ART, "annotations-pending.png"), await s.screenshot());

    // Save a copy (Ctrl+S): default name next to the source; saved here into the work folder.
    await s.keys(["Control", "s"]);
    await s.waitFor("#output");
    assert((await s.exec("return document.querySelector('#output').value")).endsWith("single-page-annotated.pdf"), "default output name");
    assert((await s.text('[role="dialog"] h2')) === "Save a copy with your changes", "save dialog title");
    await s.type("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved annotated.pdf")), "saved notice");
    await s.waitUntil(async () => (await annotCount(s)) === 0 && (await unsavedText(s)) === "", "pending annotations cleared after saving");
  });
  assert(existsSync(out), "annotated copy exists");
  const bytes = readFileSync(out).toString("latin1");
  const subtypes = [...bytes.matchAll(/\/Subtype\s*\/(Highlight|Underline|StrikeOut|Text)\b/g)].map((m) => m[1]).sort();
  assert(subtypes.join(",") === "Highlight,StrikeOut,Text,Underline", `annotation subtypes in the saved file: ${subtypes}`);
  assert(bytes.includes("(PW note 1)"), "note text saved");
  assert(sha(src) === before, "source PDF unchanged");
  const n = pdfPages(out);
  assert(n === null || n === 1, `copy keeps its page (got ${n})`);
});

await test("markup colours: chosen in Settings, used by new highlights and notes, saved in the copy, kept after restart, Default restores", async () => {
  const src = join(FIX, "single-page.pdf");
  const out = join(work, "markup-colours.pdf");
  const setColour = (s, label, v) => s.exec(`const i=document.querySelector('[aria-label="${label}"]');
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(i, '${v}');
      i.dispatchEvent(new Event('input', { bubbles: true })); return 1`);
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", ","]);
    await s.waitFor('[aria-label="Highlight color"]');
    assert((await s.exec("return document.querySelector('[aria-label=\"Highlight color\"]').value")) === "#ffd400", "built-in highlight colour shown");
    await setColour(s, "Highlight color", "#22cc88");
    await setColour(s, "Note color", "#3355ff");
    await axeCheck(s, "settings-markup-colours");
    writeFileSync(join(ART, "settings-markup-colours.png"), await s.screenshot());
    await s.click(".dialog .dialog-footer .btn.primary");
    await s.waitUntil(async () => await s.exec("return !document.querySelector('[role=dialog]')"), "settings saved");
    const w = await markSpan(s, 0, "PWMARK001");
    await s.doubleClick('[data-e2e="target"]', w * 0.3, 0);
    await s.waitUntil(async () => (await selection(s)).trim() === "PWMARK001", "word selected");
    // The context menu (the toolbar group may sit in More tools at the comfortable density).
    await s.contextClick('[data-e2e="target"]', w * 0.3, 0);
    await s.click('[role="menuitem"][data-command="annotate.highlight"]');
    await s.waitUntil(async () => (await annotCount(s, "highlight")) === 1, "highlight drawn");
    const shown = await s.exec("return getComputedStyle(document.querySelector('.annot.highlight')).backgroundColor + '|' + document.querySelector('.annot.highlight').getAttribute('style')");
    assert(/34,\s*204,\s*136|#22cc88/i.test(shown), `pending highlight drawn in the chosen colour (${shown})`);
    await s.contextClick('[data-e2e="target"]', 0, 48);
    await s.click('[role="menuitem"][data-command="annotate.note"]');
    await s.type("#note-text", "Colour note");
    await s.keys(["Control", "Enter"]);
    await s.waitUntil(async () => (await annotCount(s, "note")) === 1, "note added");
    await s.keys(["Control", "s"]);
    await s.waitFor("#output");
    await s.type("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved markup-colours.pdf")), "saved notice");
  });
  if (CLI) {
    const listed = JSON.parse(execFileSync(CLI, ["--json", "annotations", out], { encoding: "utf8" })).result.data.annotations;
    const colours = Object.fromEntries(listed.map((a) => [a.subtype, a.color]));
    assert(colours.Highlight === "#22cc88" && colours.Text === "#3355ff", `saved colours: ${JSON.stringify(colours)}`);
  }
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", ","]);
    await s.waitFor('[aria-label="Highlight color"]');
    assert((await s.exec("return document.querySelector('[aria-label=\"Highlight color\"]').value")) === "#22cc88", "chosen colour kept after restart");
    await s.exec("[...document.querySelectorAll('.dialog button')].find(b => b.title === 'Use the built-in colors for new markup').click(); return 1");
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[aria-label=\"Highlight color\"]').value")) === "#ffd400", "Default restores the built-in colour");
    await s.click(".dialog .dialog-footer .btn.primary");
    await s.waitUntil(async () => await s.exec("return !document.querySelector('[role=dialog]')"), "settings saved again");
  });
});

await test("unsaved annotations: closing the tab asks (Save focused); Cancel keeps it; discard can be undone; Discard closes", async () => {
  await withApp([join(FIX, "single-page.pdf")], async (s) => {
    await pageLoaded(s, 0);
    const w = await markSpan(s, 0, "PWMARK001");
    await s.doubleClick('[data-e2e="target"]', w * 0.3, 0);
    await s.waitUntil(async () => (await selection(s)).trim() === "PWMARK001", "word selected");
    await s.keys(["Control", "Shift", "H"]);
    await s.waitUntil(async () => (await annotCount(s, "highlight")) === 1, "highlight drawn");

    await s.keys(["Control", "w"]);
    await s.waitFor('[data-testid="unsaved-save"]');
    assert((await s.exec("return document.activeElement?.dataset.testid")) === "unsaved-save", "Save copy… has initial focus");
    await axeCheck(s, "unsaved-dialog");
    await s.keys(["Escape"]);
    await s.waitUntil(async () => (await s.exec("return !document.querySelector('[role=dialog]')")) === true, "prompt closed");
    assert((await s.exec("return document.querySelectorAll('.tab-label').length")) === 1, "Cancel keeps the tab open");
    assert((await annotCount(s, "highlight")) === 1, "and keeps the annotation");

    // Discard from the command palette, then undo the discard from its notice.
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Discard unsaved");
    await s.click('[role="option"][data-command="edit.discardChanges"]');
    await s.waitUntil(async () => (await annotCount(s)) === 0, "discarded");
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.startsWith("Discarded 1 change")), "discard notice");
    await s.click('.notice button[data-action="Undo"]');
    await s.waitUntil(async () => (await annotCount(s, "highlight")) === 1, "discard undone");

    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "w"]);
    await s.click('[data-testid="unsaved-discard"]');
    await s.waitFor(".home");
  });
});

/** Send the window a real close request (title-bar X). Linux/X11 only; see e2e/wm_close.py. */
function wmClose() {
  const win = process.platform === "win32";
  if (!win && (process.platform !== "linux" || !process.env.DISPLAY)) throw new Skip("needs an X11 display or Windows to send a window close request");
  execFileSync(win ? "python" : "python3", ["-I", fileURLToPath(new URL("./wm_close.py", import.meta.url)), "PageWeave"], { stdio: "inherit" });
}
/** Click a control that quits the app: the driver may lose the session before it can reply. */
const clickThatQuits = (s, css) =>
  s.click(css).catch((e) => {
    if (!/Session terminated without a reply|invalid session id/.test(String(e.message))) throw e;
  });
const appGone = (s) => s.waitUntil(() => s.exec("return 0").then(() => false, () => true), "app closed", 15000);
const highlightMarker = async (s) => {
  await pageLoaded(s, 0);
  const w = await markSpan(s, 0, "PWMARK001");
  await s.doubleClick('[data-e2e="target"]', w * 0.3, 0);
  await s.waitUntil(async () => (await selection(s)).trim() === "PWMARK001", "word selected");
  await s.keys(["Control", "Shift", "H"]);
  await s.waitUntil(async () => (await annotCount(s, "highlight")) === 1, "highlight drawn");
};

await test("window close: clean app closes at once; unsaved annotations ask first (Cancel keeps app, Discard quits)", async () => {
  await withApp([join(FIX, "single-page.pdf")], async (s) => {
    await pageLoaded(s, 0);
    wmClose();
    await appGone(s);
  });
  await withApp([join(FIX, "single-page.pdf")], async (s) => {
    await highlightMarker(s);
    wmClose();
    await s.waitFor('[data-testid="unsaved-save"]');
    assert((await s.exec("return document.querySelector('[role=dialog] p').textContent")).includes("1 unsaved change"), "prompt names the count");
    await s.click('[data-testid="unsaved-cancel"]');
    await s.waitUntil(async () => (await s.exec("return !document.querySelector('[role=dialog]')")) === true, "prompt closed");
    assert((await annotCount(s, "highlight")) === 1, "Cancel keeps the app and the annotation");
    wmClose();
    await clickThatQuits(s, '[data-testid="unsaved-discard"]');
    await appGone(s);
  });
});

await test("window close: Save copy… saves the annotations, then quits", async () => {
  const out = join(work, "saved-on-quit.pdf");
  await withApp([join(FIX, "single-page.pdf")], async (s) => {
    await highlightMarker(s);
    wmClose();
    await s.click('[data-testid="unsaved-save"]');
    await s.waitFor("#output");
    await s.type("#output", out);
    await clickThatQuits(s, '[data-testid="tool-start"]');
    await appGone(s);
  });
  assert(existsSync(out), "annotated copy saved before quitting");
  assert(/\/Subtype\s*\/Highlight\b/.test(readFileSync(out).toString("latin1")), "copy contains the highlight");
});

// Startup baseline (CLAUDE.md §6): the host logs ms from its entry point to UI ready and to the first
// rendered page. Measured, not budgeted; the bound below only catches hangs.
await test("startup timing: UI ready and first page logged by the host (baseline, 5 launches)", async () => {
  const marks = [];
  for (let i = 0; i < 5; i++) {
    await withApp([join(FIX, "five-pages.pdf")], async (s) => {
      await pageLoaded(s, 0);
      const dir = await s.execAsync("const done = arguments[arguments.length - 1]; window.__TAURI_INTERNALS__.invoke('log_dir').then(done, (e) => done('ERR ' + e))");
      assert(!String(dir).startsWith("ERR"), `log dir: ${dir}`);
      const log = join(dir, "pageweave.jsonl");
      const read = () => (existsSync(log) ? readFileSync(log, "utf8").trim().split("\n").map((l) => JSON.parse(l)) : []);
      // Entries since this launch's startup event.
      const run = () => {
        const all = read();
        const start = all.map((e) => e.event).lastIndexOf("startup");
        return all.slice(start).filter((e) => e.event === "timing");
      };
      await s.waitUntil(() => run().some((e) => e.fields?.mark === "first_page"), "first_page mark logged");
      const get = (m) => run().find((e) => e.fields?.mark === m);
      const ms = (e) => e?.fields?.since_start_ms;
      marks.push({ ui_ready: ms(get("ui_ready")), first_page: ms(get("first_page")) });
    });
  }
  const pct = (xs, p) => [...xs].sort((a, b) => a - b)[Math.min(xs.length - 1, Math.floor(p * xs.length))];
  const summary = {};
  for (const k of ["ui_ready", "first_page"]) {
    const xs = marks.map((m) => m[k]);
    assert(xs.every((x) => Number.isFinite(x) && x > 0), `${k} recorded each launch (${xs})`);
    summary[k] = { first: xs[0], p50: pct(xs, 0.5), max: Math.max(...xs), all: xs };
  }
  writeFileSync(join(ART, "perf-startup.json"), JSON.stringify({ platform: process.platform, ...summary }, null, 2));
  console.log(`  startup: ui_ready p50 ${summary.ui_ready.p50} ms, first_page p50 ${summary.first_page.p50} ms (first launch ${summary.first_page.first} ms)`);
  assert(summary.first_page.max < 20000, "first page appears (hang guard, not a budget)");
});

await test("interrupted task: reported at next start with its partial file removed; Run again saves the output", async () => {
  // Plant exactly what a crash mid-job leaves behind: a journal entry from a process that no longer
  // exists and its staged (uncommitted) output file. The app's real startup recovery must handle it.
  const ident = JSON.parse(readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8")).identifier;
  const base = process.platform === "win32" ? process.env.LOCALAPPDATA : process.env.XDG_DATA_HOME;
  if (!base) throw new Skip("the app data folder is not isolated for this run (XDG_DATA_HOME unset)");
  const jobsDir = join(base, ident, "jobs");
  mkdirSync(jobsDir, { recursive: true });
  const outDir = mkdtempSync(join(work, "interrupted-"));
  const out = join(outDir, "cut-short.pdf");
  const deadPid = 4294967000;
  const staged = join(outDir, `.pw-staged-${deadPid}-e2e.tmp`);
  writeFileSync(staged, "partial output");
  const entry = join(jobsDir, "job-e2e-interrupted.json");
  writeFileSync(entry, JSON.stringify({
    job_id: "e2e-interrupted", label: "extract_pages", pid: deadPid, started_unix_ms: Date.now() - 5000,
    operation: { op: "extract_pages", input: join(FIX, "five-pages.pdf"), pages: "2-3", output: out }, collision: "fail",
  }));
  await withApp([], async (s) => {
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("A task was interrupted") && t.includes("Extract pages → cut-short.pdf") && t.includes("unfinished file was removed")), "interrupted-task notice");
    assert(!existsSync(staged), "partial file removed");
    assert(!existsSync(entry), "journal entry consumed (reported once)");
    assert(!existsSync(out), "nothing was committed by the interrupted run");
    await axeCheck(s, "interrupted-notice");
    await s.click('.notice button[data-action="Run again"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved cut-short.pdf")), "run again succeeded");
  });
  assert(existsSync(out), "output saved by Run again");
  const n = pdfPages(out);
  assert(n === null || n === 2, `re-run output has 2 pages (got ${n})`);
});

await test("forms: fill text, checkbox, radio, choice and multiline; Ctrl+S saves a verified filled copy and the tab switches to it", async () => {
  const src = join(FIX, "form-fields.pdf");
  const before = sha(src);
  const out = join(work, "filled.pdf");
  const field = (name) => `[data-formlayer="0"] [data-field="${name}"]`;
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.waitFor(field("full_name"));
    assert((await s.attr(field("full_name"), "aria-label")) === "Full name", "field is labelled by /TU");
    await s.type(field("full_name"), "Ada Lovelace");
    await s.click(field("subscribe"));
    await s.click(`${field("plan")}[value="pro"]`);
    await s.click(`${field("country")} option[value="Japan"]`);
    await s.type(field("comments"), "Line one");
    await s.typeKeys("\nLine two");
    const state = () =>
      s.exec(`const q = (n) => document.querySelector('[data-formlayer="0"] [data-field="' + n + '"]');
        return [q('full_name').value, q('subscribe').checked, document.querySelector('[data-field="plan"][value="pro"]').checked, q('country').value, q('comments').value]`);
    const want = JSON.stringify(["Ada Lovelace", true, true, "Japan", "Line one\nLine two"]);
    assert(JSON.stringify(await state()) === want, `controls hold the edits (${JSON.stringify(await state())})`);
    await s.waitUntil(async () => (await unsavedText(s)) === "5 unsaved changes", "five unsaved changes");
    await axeCheck(s, "form-fields");
    writeFileSync(join(ART, "form-filled.png"), await s.screenshot());

    // Closing the tab with field edits asks first.
    await s.keys(["Control", "w"]);
    await s.waitFor('[data-testid="unsaved-cancel"]');
    await s.click('[data-testid="unsaved-cancel"]');

    // Ctrl+S from inside a field opens the save dialog.
    await s.click(field("full_name"));
    await s.keys(["Control", "s"]);
    await s.waitFor("#output");
    assert((await s.exec("return document.querySelector('#output').value")).endsWith("form-fields-filled.pdf"), "default output name");
    await s.type("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved filled.pdf")), "saved notice");
    // Save As semantics: the tab now shows the saved copy, with its own (filled) values and no pending changes.
    await s.waitUntil(async () => (await s.exec("return document.querySelector('.tab-label[aria-current=page] .name')?.textContent")) === "filled.pdf", "tab switched to the copy");
    await s.waitFor(field("full_name"));
    await s.waitUntil(async () => JSON.stringify(await state()).replace(/\\r/g, "") === want, "saved copy shows the filled values");
    assert((await unsavedText(s)) === "", "no unsaved changes after saving");
  });
  assert(existsSync(out), "filled copy exists");
  assert(sha(src) === before, "source PDF unchanged");
  if (CLI) {
    const listing = execFileSync(CLI, ["fields", out], { encoding: "utf8" });
    for (const needle of ['full_name  (text) = "Ada Lovelace"', "subscribe  (checkbox) = checked", "options: basic [pro]", 'country  (combo) = "Japan"']) {
      assert(listing.includes(needle), `CLI sees "${needle}" in the saved file:\n${listing}`);
    }
  }
});

await test("XFA forms: a hybrid form (like government forms) fills and saves without its stale XFA layer; a dynamic one is not offered", async () => {
  const field = (name) => `[data-formlayer="0"] [data-field="${name}"]`;
  const out = join(work, "xfa-filled.pdf");
  await withApp([join(FIX, "form-xfa-hybrid.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.waitFor(field("full_name"));
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("XFA layer")), "the XFA layer is explained");
    await s.type(field("full_name"), "Ada Lovelace");
    await s.keys(["Control", "s"]);
    await s.waitFor("#output");
    await s.type("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved xfa-filled.pdf")), "saved notice");
  });
  if (CLI) {
    const f = JSON.parse(execFileSync(CLI, ["--json", "fields", out], { encoding: "utf8" })).result.data;
    assert(!f.has_xfa && f.fields.find((x) => x.name === "full_name").value === "Ada Lovelace", `saved: xfa ${f.has_xfa}`);
  }
  await withApp([join(FIX, "form-xfa-dynamic.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("dynamic XFA form")), "dynamic XFA explained");
    assert(!(await s.exec(`return !!document.querySelector('${field("full_name")}')`)), "no fillable controls for a dynamic XFA form");
  });
});

await test("real IRS Form 1040 (test corpus): fill name fields in the app, save, values read back", async () => {
  const src = resolve(fileURLToPath(new URL("../../target/corpus/real/irs-f1040.pdf", import.meta.url)));
  if (!existsSync(src) || !CLI) throw new Skip("test corpus not built (python3 tools/corpus/build.py) or no CLI");
  const names = JSON.parse(execFileSync(CLI, ["--json", "fields", src], { encoding: "utf8" })).result.data.fields.filter((f) => f.kind === "text" && f.widgets[0]?.page === 0).slice(0, 2).map((f) => f.name);
  const out = join(work, "f1040-filled.pdf");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    for (const [i, n] of names.entries()) {
      const sel = `[data-formlayer="0"] [data-field="${n}"]`;
      await s.waitFor(sel);
      await s.type(sel, ["Ada", "Lovelace"][i]);
    }
    await s.keys(["Control", "s"]);
    await s.waitFor("#output");
    await s.type("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved f1040-filled.pdf")), "saved notice", 60000);
  });
  const f = JSON.parse(execFileSync(CLI, ["--json", "fields", out], { encoding: "utf8" })).result.data;
  const got = names.map((n) => f.fields.find((x) => x.name === n).value);
  assert(JSON.stringify(got) === '["Ada","Lovelace"]' && !f.has_xfa, `saved ${JSON.stringify(got)} xfa ${f.has_xfa}`);
});

await test("form data: export XFDF with unsaved values; import JSON with preview, Undo, then save a verified copy", async () => {
  const src = join(FIX, "form-nested.pdf");
  const before = sha(src);
  const dir = mkdtempSync(join(work, "formdata-"));
  const xfdf = join(dir, "nested.xfdf");
  const json = join(dir, "in.json");
  const out = join(dir, "nested-filled.pdf");
  writeFileSync(json, JSON.stringify({ "applicant.name": "Grace Hopper", "applicant.address.city": "Arlington", colour: "Blue", agree: "Yes", ref_no: "X", not_here: "y" }));
  const field = (name) => `[data-formlayer="0"] [data-field="${name}"]`;
  const palette = async (s, text, id) => {
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', text);
    await s.click(`[role="option"][data-command="${id}"]`);
  };
  const values = (s) =>
    s.exec(`const q = (n) => document.querySelector('[data-formlayer="0"] [data-field="' + n + '"]');
      return [q('applicant.name').value, q('applicant.address.city').value, q('colour').value, q('agree').checked]`);
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.waitFor(field("applicant.name"));
    await s.type(field("applicant.name"), "Ada Lovelace");
    await s.waitUntil(async () => (await unsavedText(s)) === "1 unsaved change", "one unsaved change");

    // Export: XFDF by default, unsaved value included.
    await s.click('[data-testid="viewer"]');
    await palette(s, "Export form data", "file.exportFormData");
    await s.waitFor("#formdata-export-output");
    assert((await s.exec("return document.querySelector('#formdata-export-output').value")).endsWith("form-nested-data.xfdf"), "default name");
    assert((await s.text('[data-testid="formdata-export-note"]')) === "Includes 1 unsaved change.", "unsaved values noted");
    await s.type("#formdata-export-output", xfdf);
    await axeCheck(s, "export-form-data-dialog");
    await s.click('[data-testid="formdata-export-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved nested.xfdf") && t.includes("5 field values")), "export notice");
    const text = readFileSync(xfdf, "utf8");
    assert(text.includes('<field name="applicant">') && text.includes("<value>Ada Lovelace</value>"), `nested XFDF with the unsaved value:\n${text}`);
    assert(text.includes("<value>R-1001</value>") && !text.includes("submit"), "read-only value kept, push button left out");

    // Import: preview, fill, Undo.
    await palette(s, "Import form data", "file.importFormData");
    await s.waitFor("#formdata-import-path");
    assert((await s.attr('[data-testid="formdata-import-apply"]', "title")) === "Choose a form data file.", "fill disabled until a file is read");
    await s.type("#formdata-import-path", json);
    await s.click('[data-testid="formdata-import-read"]');
    await s.waitUntil(async () => (await s.text('[data-testid="formdata-import-summary"]')) === "4 fields to fill · 2 not used", "preview summary");
    await s.click('[data-testid="formdata-import-unused"] summary');
    await s.waitUntil(() => s.exec("return document.querySelector('[data-testid=formdata-import-unused]').open"), "not-used list opens on click");
    // WebKitWebDriver's element text leaves out <details> content; read the DOM text.
    const unused = await s.exec("return document.querySelector('[data-testid=formdata-import-unused]').textContent");
    const order = await s.exec("return [...document.querySelectorAll('[data-testid=formdata-import-changes] .formdata-name')].map(e => e.textContent)");
    assert(JSON.stringify(order) === JSON.stringify(["Applicant name", "City", "Colour", "Agree"]), `changes listed in form order: ${JSON.stringify(order)}`);
    assert(unused.includes("ref_no") && unused.includes("read-only") && unused.includes("not_here"), `not-used list: ${unused}`);
    await axeCheck(s, "import-form-data-preview");
    writeFileSync(join(ART, "import-form-data.png"), await s.screenshot());
    await s.click('[data-testid="formdata-import-apply"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Filled 4 fields from in.json")), "filled notice");
    const filled = JSON.stringify(["Grace Hopper", "Arlington", "Blue", true]);
    await s.waitUntil(async () => JSON.stringify(await values(s)) === filled, "controls show the imported values");
    await s.waitUntil(async () => (await unsavedText(s)) === "4 unsaved changes", "four unsaved changes");
    await s.click('.notice button[data-action="Undo"]');
    await s.waitUntil(async () => JSON.stringify(await values(s)) === JSON.stringify(["Ada Lovelace", "", "", false]), "Undo restores the earlier edits");
    assert((await s.exec("return document.querySelector('[data-formlayer=\"0\"] [data-field=\"colour\"]').selectedOptions[0]?.hidden ?? true")) === true, "list box shows no choice after Undo (none in the document)");
    await s.waitUntil(async () => (await unsavedText(s)) === "1 unsaved change", "back to one unsaved change");

    // Import again (Enter reads, Enter does not fill a stale preview), then save a copy.
    await palette(s, "Import form data", "file.importFormData");
    await s.waitFor("#formdata-import-path");
    await s.type("#formdata-import-path", json);
    await s.keys(["Enter"]);
    await s.waitUntil(async () => (await s.text('[data-testid="formdata-import-summary"]')).startsWith("4 fields to fill"), "preview again");
    await s.click('[data-testid="formdata-import-apply"]');
    await s.waitUntil(async () => JSON.stringify(await values(s)) === filled, "values imported again");
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "s"]);
    await s.waitFor("#output");
    await s.type("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved nested-filled.pdf")), "saved notice");
  });
  assert(sha(src) === before, "source PDF unchanged");
  assert(existsSync(out), "filled copy exists");
  if (CLI) {
    const f = JSON.parse(execFileSync(CLI, ["--json", "fields", out], { encoding: "utf8" })).result.data.fields;
    const v = Object.fromEntries(f.map((x) => [x.name, x.value]));
    assert(v["applicant.name"] === "Grace Hopper" && v["applicant.address.city"] === "Arlington" && v.colour === "Blue" && v.agree === "Yes" && v.ref_no === "R-1001", `CLI reads the saved values: ${JSON.stringify(v)}`);
  }
});

await test("clear form fields: text and checkboxes emptied as unsaved edits, choices kept and said so; Undo", async () => {
  const field = (name) => `[data-formlayer="0"] [data-field="${name}"]`;
  await withApp([join(FIX, "form-nested.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.waitFor(field("applicant.name"));
    await s.type(field("applicant.name"), "Ada");
    await s.click(field("agree"));
    await s.click(`${field("colour")} option[value="Green"]`);
    await s.waitUntil(async () => (await unsavedText(s)) === "3 unsaved changes", "three edits");
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Clear form");
    await s.click('[role="option"][data-command="edit.clearForm"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Cleared 2 fields") && t.includes("1 choice field keeps its option")), "cleared notice");
    const state = () => s.exec(`const q = (n) => document.querySelector('[data-formlayer="0"] [data-field="' + n + '"]'); return [q('applicant.name').value, q('agree').checked, q('colour').value, q('ref_no').value]`);
    assert(JSON.stringify(await state()) === JSON.stringify(["", false, "Green", "R-1001"]), `after clear: ${JSON.stringify(await state())}`);
    await s.waitUntil(async () => (await unsavedText(s)) === "1 unsaved change", "only the kept choice remains an edit");
    await s.click('.notice button[data-action="Undo"]');
    await s.waitUntil(async () => JSON.stringify(await state()) === JSON.stringify(["Ada", true, "Green", "R-1001"]), "Undo restores");
  });
});

await test("reset form: fields return to the document's default values as unsaved edits; Undo; saved copy verified", async () => {
  const src = join(FIX, "form-defaults.pdf");
  const before = sha(src);
  const dir = mkdtempSync(join(work, "reset-"));
  const out = join(dir, "reset.pdf");
  const field = (name) => `[data-formlayer="0"] [data-field="${name}"]`;
  const state = (s) =>
    s.exec(`const q = (n) => document.querySelector('[data-formlayer="0"] [data-field="' + n + '"]');
      const plan = [...document.querySelectorAll('[data-formlayer="0"] [data-field="plan"]')].find((r) => r.checked)?.value ?? "";
      return [q('full_name').value, q('subscribe').checked, plan, q('country').value, q('comments').value]`);
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.waitFor(field("full_name"));
    await s.type(field("full_name"), "Ada");
    await s.type(field("comments"), "note");
    await s.waitUntil(async () => (await unsavedText(s)) === "2 unsaved changes", "two edits");
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Reset form");
    await s.click('[role="option"][data-command="edit.resetForm"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Reset 5 fields to default values")), "reset notice");
    const reset = JSON.stringify(["Jane Doe", true, "pro", "Japan", ""]);
    await s.waitUntil(async () => JSON.stringify(await state(s)) === reset, "controls show the defaults");
    await s.waitUntil(async () => (await unsavedText(s)) === "4 unsaved changes", "four fields differ from the file");
    await s.click('.notice button[data-action="Undo"]');
    await s.waitUntil(async () => JSON.stringify(await state(s)) === JSON.stringify(["Ada", false, "", "Canada", "note"]), "Undo restores the typed values");
    // Again, then once more: nothing left to reset.
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Reset form");
    await s.click('[role="option"][data-command="edit.resetForm"]');
    await s.waitUntil(async () => JSON.stringify(await state(s)) === reset, "reset again");
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Reset form");
    await s.click('[role="option"][data-command="edit.resetForm"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("already shows its default values")), "nothing to reset");
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "s"]);
    await s.waitFor("#output");
    await s.type("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved reset.pdf")), "saved notice");
  });
  assert(sha(src) === before, "source PDF unchanged");
  if (CLI) {
    const f = JSON.parse(execFileSync(CLI, ["--json", "fields", out], { encoding: "utf8" })).result.data.fields;
    const v = Object.fromEntries(f.map((x) => [x.name, x.kind === "radio" ? x.widgets.find((w) => w.checked)?.export_value : x.kind === "checkbox" ? x.widgets[0].checked : x.value]));
    assert(v.full_name === "Jane Doe" && v.subscribe === true && v.plan === "pro" && v.country === "Japan" && (v.comments ?? "") === "", `CLI reads the reset values: ${JSON.stringify(v)}`);
  }
});

await test("split: by bookmarks (parts named after them) and by size (every part under the limit)", async () => {
  const dir = mkdtempSync(join(work, "split-"));
  const openSplit = async (s) => {
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Split document");
    await s.click('[role="option"][data-command="pages.split"]');
    await s.waitFor("#output");
  };
  const radio = (s, label) => s.exec(`[...document.querySelectorAll('[role=dialog] [role=radio]')].find(b => b.textContent.trim() === ${JSON.stringify(label)}).click()`);
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await openSplit(s);
    await radio(s, "By bookmarks");
    await s.waitUntil(async () => (await s.text('[data-testid="split-bookmarks-note"]')) === "Creates 5 files, named after the bookmarks.", "bookmark preview");
    await s.type("#output", dir);
    await axeCheck(s, "split-bookmarks");
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved 5 files")), "saved notice");
  });
  for (let i = 1; i <= 5; i++) assert(existsSync(join(dir, `five-pages_0${i}_Chapter ${i}.pdf`)), `part ${i} named after its bookmark`);
  if (CLI) assert(pdfPages(join(dir, "five-pages_03_Chapter 3.pdf")) === 1, "one page per chapter");

  const dir2 = mkdtempSync(join(work, "split-size-"));
  await withApp([join(FIX, "photo-heavy.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await openSplit(s);
    await radio(s, "By size");
    await s.type("#split-size", "5");
    await s.exec("const u = document.querySelector('[aria-label=\"Size unit\"]'); u.value = 'KB'; u.dispatchEvent(new Event('change', { bubbles: true }))");
    await s.waitUntil(async () => (await s.attr('[data-testid="tool-start"]', "title")) === "Choose a size of at least 10 KB.", "tiny limit refused");
    await s.type("#split-size", "300");
    await s.type("#output", dir2);
    await s.waitUntil(() => s.exec("return !document.querySelector('[data-testid=tool-start]').hasAttribute('aria-disabled')"), "limit accepted");
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => /Saved \d+ files/.test(t)), "saved notice");
  });
  const parts = readdirSync(dir2).filter((f) => f.endsWith(".pdf"));
  assert(parts.length >= 2, `several parts (${parts.length})`);
  for (const f of parts) {
    const size = statSync(join(dir2, f)).size;
    assert(size <= 300 * 1024 || pdfPages(join(dir2, f)) === 1, `${f} is ${size} bytes`);
  }
});

await test("organize pages: keyboard reorder, rotate, duplicate; saved file verified independently", async () => {
  const out = join(work, "organized.pdf");
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "Shift", "O"]);
    await s.waitFor(".organize-grid");
    assert((await s.exec("return document.activeElement?.classList.contains('organize-grid')")) === true, "page grid has focus");
    assert((await s.attr('[data-testid="organize-save"]', "aria-disabled")) === "true", "Save disabled until something changes");
    const refs = () => s.exec("return [...document.querySelectorAll('[data-page-ref]')].map(t => t.dataset.pageRef)");
    await s.keys(["End"]);
    for (let i = 0; i < 4; i++) await s.keys(["Control", "ArrowLeft"]); // page 5 to the front
    await s.keys(["r"]); // turn it right
    await s.keys(["Control", "d"]); // and duplicate it
    const got = JSON.stringify(await refs());
    assert(got === JSON.stringify(["0:4:90", "0:4:90", "0:0:0", "0:1:0", "0:2:0", "0:3:0"]), `tiles after keyboard edits: ${got}`);
    await axeCheck(s, "organize-dialog");
    writeFileSync(join(ART, "organize.png"), await s.screenshot());
    await s.type("#organize-output", out);
    await s.click('[data-testid="organize-save"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved organized.pdf")), "saved notice");
  });
  assert(existsSync(out), "organized file exists");
  if (CLI) {
    const info = JSON.parse(execFileSync(CLI, ["--json", "inspect", out], { encoding: "utf8" })).result.data;
    assert(info.page_count === 6, `6 pages (got ${info.page_count})`);
    assert(JSON.stringify(info.pages.map((p) => p.rotation)) === "[90,90,0,0,0,0]", `rotations ${JSON.stringify(info.pages.map((p) => p.rotation))}`);
    const text = JSON.parse(execFileSync(CLI, ["--json", "text", out], { encoding: "utf8" })).result.data.pages.map((p) => p.text);
    const marks = text.map((t) => (t.match(/PWMARK\d{3}/) ?? ["?"])[0]);
    assert(JSON.stringify(marks) === JSON.stringify(["PWMARK005", "PWMARK005", "PWMARK001", "PWMARK002", "PWMARK003", "PWMARK004"]), `page order ${marks}`);
  }
});

await test("page numbers: template preview, validation, saved numbers verified by the CLI", async () => {
  const out = join(work, "numbered.pdf");
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Page numbers");
    await s.click('[role="option"][data-command="pages.stamp"]');
    await s.waitFor("#stamp-text");
    assert((await s.text("#stamp-help")).includes("“Page 1 of 5”"), "live example of the default template");
    await s.type("#stamp-text", "Doc {page}");
    await s.type("#stamp-digits", "3");
    await s.waitUntil(async () => (await s.text("#stamp-help")).includes("“Doc 001”"), "preview follows the template and digits");
    await s.type("#stamp-size", "200");
    await s.waitUntil(async () => (await s.attr('[data-testid="tool-start"]', "title")) === "Use a text size from 4 to 72.", "invalid size blocks Start with a reason");
    await s.type("#stamp-size", "11");
    await axeCheck(s, "stamp-dialog");
    await s.type("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved numbered.pdf")), "saved notice");
  });
  assert(existsSync(out), "numbered file exists");
  if (CLI) {
    const pages = JSON.parse(execFileSync(CLI, ["--json", "text", out], { encoding: "utf8" })).result.data.pages.map((p) => p.text.replace(/\s+/g, ""));
    pages.forEach((t, i) => assert(t.includes(`Doc${String(i + 1).padStart(3, "0")}`), `page ${i + 1} carries its number: ${t.slice(-40)}`));
  }
});

await test("page reload: open documents come back as tabs and render", async () => {
  await withApp([join(FIX, "single-page.pdf"), join(FIX, "five-pages.pdf")], async (s) => {
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('.tab-label').length")) === 2, "two tabs");
    await pageLoaded(s, 0);
    // reload() returns before navigation starts: mark the old page so its tabs are not counted.
    await s.exec("window.__beforeReload = true; location.reload(); return 1");
    await s.waitUntil(
      async () => (await s.exec("return window.__beforeReload ? -1 : document.querySelectorAll('.tab-label .name').length").catch(() => -1)) === 2,
      "both tabs restored after reload",
      20000,
    );
    const names = await s.exec("return [...document.querySelectorAll('.tab-label .name')].map(n => n.textContent)");
    assert(JSON.stringify(names) === JSON.stringify(["single-page.pdf", "five-pages.pdf"]), `tab order kept (${names})`);
    await pageLoaded(s, 0);
    assert((await s.exec("return document.querySelector('.tab-label[aria-current=page] .name').textContent")) === "five-pages.pdf", "last document active");
  });
});

await test("export pages as images: page range and resolution; PNG files written with the right size", async () => {
  const dir = mkdtempSync(join(work, "images-"));
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Export pages");
    await s.click('[role="option"][data-command="pages.exportImages"]');
    await s.waitFor("#export-dpi");
    await s.click('[role="dialog"] input[type="checkbox"]'); // untick "All pages"
    await s.type("#pages", "2-3");
    await s.waitUntil(async () => (await s.text("#pages-msg")) === "2 pages selected", "range validated");
    await s.click('#export-dpi option[value="72"]');
    await s.type("#output", dir);
    await axeCheck(s, "export-images-dialog");
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved 2 files")), "saved notice");
  });
  const png = (n) => readFileSync(join(dir, `five-pages_p00${n}.png`));
  for (const n of [2, 3]) {
    const b = png(n);
    assert(b.subarray(1, 4).toString() === "PNG", `page ${n} is a PNG`);
    assert(b.readUInt32BE(16) === 612 && b.readUInt32BE(20) === 792, `page ${n} is 612×792 at 72 dpi (got ${b.readUInt32BE(16)}×${b.readUInt32BE(20)})`);
  }
  assert(!existsSync(join(dir, "five-pages_p001.png")), "only the chosen pages");
});

await test("export as text: chosen pages saved as UTF-8 text with page sections; existing file kept until Replace", async () => {
  const dir = mkdtempSync(join(work, "text-"));
  const out = join(dir, "five.txt");
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Export as text");
    await s.click('[role="option"][data-command="pages.exportText"]');
    await s.waitFor("#output");
    assert((await s.exec("return document.querySelector('#output').value")).endsWith("five-pages.txt"), "default name is the document's with .txt");
    await s.click('[role="dialog"] input[type="checkbox"]'); // untick "All pages"
    await s.type("#pages", "2, 4");
    await s.waitUntil(async () => (await s.text("#pages-msg")) === "2 pages selected", "range validated");
    await s.type("#output", out);
    await axeCheck(s, "export-text-dialog");
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved five.txt")), "saved notice");
    const text = readFileSync(out, "utf8");
    assert(text.includes("--- Page 2 ---\r\n") && text.includes("PWMARK002") && text.includes("PWMARK004"), "chosen pages written");
    assert(!text.includes("PWMARK001") && !text.includes("PWMARK003"), "other pages left out");
    // A second export to the same name asks before replacing.
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Export as text");
    await s.click('[role="option"][data-command="pages.exportText"]');
    await s.waitFor("#output");
    await s.type("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(
      async () => await s.exec("return [...document.querySelectorAll('[role=dialog] .btn.danger')].some(b => b.textContent.includes('Replace'))"),
      "conflict offered",
    );
    assert(readFileSync(out, "utf8") === text, "file untouched until Replace");
    await s.exec("[...document.querySelectorAll('[role=dialog] .btn.danger')].find(b => b.textContent.includes('Replace')).click()");
    await s.waitUntil(async () => readFileSync(out, "utf8").includes("PWMARK001"), "replaced with all pages");
  });
});

/** One part of a ZIP package (the Word file), read with Node's own inflate. */
function zipPart(file, name) {
  const b = readFileSync(file);
  const eocd = b.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  assert(eocd >= 0, `${file} is a ZIP package`);
  let p = b.readUInt32LE(eocd + 16);
  const n = b.readUInt16LE(eocd + 10);
  for (let i = 0; i < n; i++) {
    const method = b.readUInt16LE(p + 10);
    const size = b.readUInt32LE(p + 20);
    const nameLen = b.readUInt16LE(p + 28);
    const extra = b.readUInt16LE(p + 30);
    const comment = b.readUInt16LE(p + 32);
    const local = b.readUInt32LE(p + 42);
    const entry = b.toString("utf8", p + 46, p + 46 + nameLen);
    if (entry === name) {
      const start = local + 30 + b.readUInt16LE(local + 26) + b.readUInt16LE(local + 28);
      const data = b.subarray(start, start + size);
      return (method === 8 ? inflateRawSync(data) : data).toString("utf8");
    }
    p += 46 + nameLen + extra + comment;
  }
  throw new Error(`${name} not in ${file}`);
}

await test("convert to Word: palette and page tools; editable Word file with columns, headings, table, link; notes reported; source unchanged", async () => {
  const src = join(FIX, "article.pdf");
  const before = sha(src);
  const out = join(work, "article-converted.docx");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Convert to Word");
    await s.click('[role="option"][data-command="file.toWord"]');
    await s.waitFor("#output");
    assert((await s.exec("return document.querySelector('#output').value")).endsWith("article.docx"), "default name is the document's with .docx");
    await s.type("#output", out);
    await axeCheck(s, "convert-to-word-dialog");
    writeFileSync(join(ART, "convert-to-word.png"), await s.screenshot());
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved article-converted.docx")), "saved notice");
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("fonts Word may not have")), "font substitution reported");
    // Only PDFs open in PageWeave: the notice offers Show in folder, not Open.
    assert(await s.exec("return !!document.querySelector('.notice button[data-action=\"Show in folder\"]')"), "show in folder offered");
  });
  assert(sha(src) === before, "source PDF unchanged");
  const xml = zipPart(out, "word/document.xml");
  assert(xml.includes('<w:cols w:num="2"'), "two columns");
  assert((xml.match(/<w:pStyle w:val="Heading/g) ?? []).length >= 5, "headings styled");
  assert(xml.includes("1 Introduction") && xml.includes("rivers.example.org") && xml.includes("Station"), "text carried over");
  assert(xml.includes("<w:tbl>"), "the ruled table is a Word table");
  assert(zipPart(out, "word/_rels/document.xml.rels").includes("https://rivers.example.org/data"), "link kept");
});

await test("open an Office file: a Word file passed to the app opens the convert dialog; converted with LibreOffice and opened; macros off", async () => {
  // A Word file made by PageWeave's own export of the article.
  const docx = join(work, "office-input.docx");
  if (CLI) execFileSync(CLI, ["to-docx", join(FIX, "article.pdf"), "-o", docx]);
  else throw new Skip("needs the CLI to make the Word file");
  const before = sha(docx);
  const out = join(work, "office-input.pdf");
  await withApp([], async (s) => {
    const engine = await s.exec("return window.__TAURI_INTERNALS__.invoke('office_engine')");
    if (!engine) throw new Skip("LibreOffice is not installed on this machine");
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Open a Word");
    await s.click('[role="option"][data-command="file.openOffice"]');
    await s.waitFor('[data-testid="office-start"]');
    assert((await s.exec("return document.querySelector('[data-testid=office-start]').getAttribute('title')")) === "Choose a file first.", "needs a file");
    await s.keys(["Escape"]);
  });
  await withApp([docx], async (s) => {
    await s.waitFor('[data-testid="office-start"]');
    assert((await s.text('[data-testid="office-file"]')) === "office-input.docx", "the file passed to the app");
    assert((await s.exec("return document.querySelector('#office-output').value")).endsWith("office-input.pdf"), "PDF next to it");
    await s.exec("document.querySelector('#office-output').value = ''");
    await s.type("#office-output", out);
    await axeCheck(s, "open-office-dialog");
    writeFileSync(join(ART, "open-office.png"), await s.screenshot());
    await s.click('[data-testid="office-start"]');
    await s.waitUntil(async () => (await s.exec("return [...document.querySelectorAll('.tab-label')].map(t => t.textContent).join('|')")).includes("office-input.pdf"), "the converted PDF opened", 120000);
    await pageLoaded(s, 0);
  });
  assert(sha(docx) === before, "the Word file is not changed");
  assert(existsSync(out), "PDF written");
});

await test("convert to PowerPoint: one slide per page with text boxes, table and picture; source unchanged", async () => {
  const src = join(FIX, "article.pdf");
  const before = sha(src);
  const out = join(work, "article-slides.pptx");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Convert to PowerPoint");
    await s.click('[role="option"][data-command="file.toPowerPoint"]');
    await s.waitFor("#output");
    assert((await s.exec("return document.querySelector('#output').value")).endsWith("article.pptx"), "default name");
    await s.type("#output", out);
    await axeCheck(s, "convert-to-powerpoint-dialog");
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved article-slides.pptx")), "saved notice");
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("scaled to the slide size")), "page size note");
  });
  assert(sha(src) === before, "source unchanged");
  const s1 = zipPart(out, "ppt/slides/slide1.xml");
  const s2 = zipPart(out, "ppt/slides/slide2.xml");
  zipPart(out, "ppt/slides/slide3.xml");
  assert(s1.includes("1 Introduction") && s1.includes("<p:pic>") && s1.includes("hlinkClick"), "slide 1: text, picture, link");
  assert(s2.includes("<a:tbl>") && s2.includes("Station"), "slide 2: table");
});

await test("translate side by side: page 1 into German as a PDF with the original next to its translation, with a glossary; option only for PDF", async () => {
  const src = join(FIX, "five-pages.pdf");
  const out = join(work, "five-pages-de-side.pdf");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Translate");
    await s.click('[role="option"][data-command="file.translate"]');
    await s.waitFor("#translate-to");
    await s.exec("const t = document.querySelector('#translate-to'); t.value = 'de'; t.dispatchEvent(new Event('change', { bubbles: true }));");
    await s.waitFor('[data-testid="translate-side-by-side"]');
    // Word and text output have no side-by-side option.
    await s.click('[data-testid="translate-format-docx"]');
    await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('[data-testid=translate-side-by-side]')")), "no option for Word");
    await s.click('[data-testid="translate-format-pdf"]');
    await s.click('[data-testid="translate-side-by-side"]');
    await s.exec("const c = [...document.querySelectorAll('[role=dialog] input[type=checkbox]')].find((x) => x.checked && x.dataset.testid !== 'translate-side-by-side'); if (c) c.click();");
    await s.type("#pages", "1");
    await s.waitUntil(async () => (await s.text("#pages-msg")) === "1 page selected", "page 1 chosen");
    await s.replaceText("#output", out);
    // Glossary: a line without "=" blocks Translate with its reason; then a valid term.
    await s.type("#translate-glossary", "fox");
    await s.waitUntil(async () => (await s.text('[data-testid="translate-glossary-msg"]')).includes("Glossary line 1"), "glossary line refused");
    assert((await s.exec("return document.querySelector('[data-testid=tool-start]').title")).includes("Glossary line 1"), "start blocked with the reason");
    await s.replaceText("#translate-glossary", "fox = Wolf");
    await s.waitUntil(async () => (await s.text('[data-testid="translate-glossary-msg"]')).startsWith("1 term"), "one term");
    await axeCheck(s, "translate-side-by-side");
    writeFileSync(join(ART, "translate-glossary.png"), await s.screenshot());
    await s.waitUntil(async () => !["Recognizing the document's language…", "Download the language first."].includes(await s.exec("return document.querySelector('[data-testid=tool-start]').title")), "ready to translate", 60000);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved five-pages-de-side.pdf")), "saved notice", 180000);
  });
  if (CLI) {
    const info = JSON.parse(execFileSync(CLI, ["--json", "inspect", out], { encoding: "utf8" })).result.data;
    const orig = JSON.parse(execFileSync(CLI, ["--json", "inspect", src], { encoding: "utf8" })).result.data;
    assert(info.page_count === 1 && Math.abs(info.pages[0].width_pt - 2 * orig.pages[0].width_pt) < 0.5, `one sheet twice the page's width: ${JSON.stringify(info.pages)}`);
    const text = JSON.parse(execFileSync(CLI, ["--json", "text", out], { encoding: "utf8" })).result.data.pages[0].text;
    const original = JSON.parse(execFileSync(CLI, ["--json", "text", "--pages", "1", src], { encoding: "utf8" })).result.data.pages[0].text;
    const firstLine = original.split("\n").map((l) => l.trim()).find((l) => l.length > 10);
    assert(text.includes(firstLine), `original kept on the sheet: ${firstLine}`);
    assert(/\b(und|der|die|das|Seite|Kapitel)\b/i.test(text), `German on the sheet: ${text.slice(0, 400)}`);
    assert(text.includes("Wolf") && !text.includes("Fuchs"), `the glossary's translation used: ${text.slice(0, 400)}`);
  }
});

await test("translate: page to German as Word (layout kept), language detected; six languages, pairs direct; source unchanged", async () => {
  const src = join(FIX, "five-pages.pdf");
  const before = sha(src);
  const out = join(work, "five-pages-de.docx");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Translate");
    await s.click('[role="option"][data-command="file.translate"]');
    await s.waitFor("#translate-to");
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('#translate-to option').length")) === 6, "six languages offered");
    // French → German is a direct pair (D-027): no "through English" notice.
    await s.exec("const f = document.querySelector('#translate-from'); f.value = 'fr'; f.dispatchEvent(new Event('change', { bubbles: true }));");
    await s.exec("const t = document.querySelector('#translate-to'); t.value = 'de'; t.dispatchEvent(new Event('change', { bubbles: true }));");
    await s.waitUntil(async () => (await s.exec("return document.querySelector('#translate-to').value")) === "de", "French to German chosen");
    assert(!(await s.exec("return !!document.querySelector('[data-testid=translate-via]')")), "direct pair: no route through English");
    // Same language on both sides is refused with a reason.
    await s.exec("const f = document.querySelector('#translate-from'); f.value = 'de'; f.dispatchEvent(new Event('change', { bubbles: true }));");
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[data-testid=tool-start]').title")) === "Choose two different languages.", "same language refused");
    // Detect the language, translate page 1 into German as a Word document.
    await s.exec("const f = document.querySelector('#translate-from'); f.value = ''; f.dispatchEvent(new Event('change', { bubbles: true }));");
    // A PDF in the same layout is the default; this run saves a Word document.
    assert((await s.exec("return document.querySelector('#output').value")).endsWith("five-pages-de.pdf"), "default name (same-layout PDF)");
    await s.click('[data-testid="translate-format-docx"]');
    assert((await s.exec("return document.querySelector('#output').value")).endsWith("five-pages-de.docx"), "Word name");
    await s.exec("const c = [...document.querySelectorAll('[role=dialog] input[type=checkbox]')].find((x) => x.checked); if (c) c.click();");
    await s.type("#pages", "1");
    await s.waitUntil(async () => (await s.text("#pages-msg")) === "1 page selected", "page 1 chosen");
    await s.type("#output", out);
    await axeCheck(s, "translate-dialog");
    // Installed copies have no models until first use (D-023): the dialog offers the one-time
    // download, which runs for real here (Windows CI); development builds use the fetched models.
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[data-testid=tool-start]').title")) !== "Recognizing the document's language…", "language recognized", 60000);
    if (await s.exec("return !!document.querySelector('[data-testid=translate-download]')")) {
      assert(/one-time download/.test(await s.text('[data-testid="translate-download"]')), "download offered with its size");
      assert((await s.exec("return document.querySelector('[data-testid=tool-start]').title")) === "Download the language first.", "start waits for the download");
      await s.click('[data-testid="translate-download-start"]');
      await s.waitFor('[data-testid="translate-download-cancel"]');
      await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('[data-testid=translate-download]')")), "language downloaded", 600000);
      console.log("  note: the English → German model was downloaded and verified in the app");
    }
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved five-pages-de.docx")), "saved notice", 180000);
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Machine translation from English to German")), "review note");
  });
  assert(sha(src) === before, "source unchanged");
  const doc = zipPart(out, "word/document.xml");
  assert(/fuchs/i.test(doc) && doc.includes("PWMARK001") && doc.includes("PageWeave"), `German text with the marker and name kept: ${doc.slice(0, 600)}`);
});

await test("read aloud: words highlighted as heard, pause/resume, next sentence, speed saved, stop; save as audio", async () => {
  const src = join(FIX, "article.pdf");
  const out = join(work, "article-page2.wav");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.click('[data-testid="viewer"]');
    // Shortcut starts reading at the current page.
    await s.keys(["Control", "Shift", "l"]);
    await s.waitFor('[data-testid="readbar"]');
    await axeCheck(s, "read-aloud-bar");
    await s.waitUntil(async () => ["playing", "error"].includes(await s.exec("return document.querySelector('[data-testid=readbar]').dataset.status")), "reading starts", 30000);
    const status = await s.exec("return document.querySelector('[data-testid=readbar]').dataset.status");
    assert(status === "playing", `reading plays (status ${status}: ${await s.text('[data-testid="readbar"] [role="status"]')})`);
    // The highlight moves with the audio.
    await s.waitFor('.reading-mark[data-reading="0"]', 15000);
    writeFileSync(join(ART, "read-aloud.png"), await s.screenshot());
    const w1 = Number(await s.exec("return document.querySelector('[data-testid=readbar]').dataset.where"));
    await s.waitUntil(async () => Number(await s.exec("return document.querySelector('[data-testid=readbar]').dataset.where")) > w1, "highlight advances", 15000);
    // Pause stops the highlight; resume continues.
    await s.click('[data-testid="read-toggle"]');
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[data-testid=readbar]').dataset.status")) === "paused", "paused");
    const w2 = Number(await s.exec("return document.querySelector('[data-testid=readbar]').dataset.where"));
    await new Promise((r) => setTimeout(r, 800));
    assert(Number(await s.exec("return document.querySelector('[data-testid=readbar]').dataset.where")) === w2, "no movement while paused");
    await s.click('[data-testid="read-toggle"]');
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[data-testid=readbar]').dataset.status")) === "playing", "resumed");
    // Next sentence jumps ahead.
    await s.click('[data-testid="read-next"]');
    await s.waitUntil(async () => Number(await s.exec("return document.querySelector('[data-testid=readbar]').dataset.where")) > w2, "next sentence");
    // A new speed restarts the sentence and is saved.
    await s.exec("const e = document.querySelector('#read-rate'); e.value = '1.5'; e.dispatchEvent(new Event('change', { bubbles: true }));");
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[data-testid=readbar]').dataset.status")) === "playing", "playing at 1.5x", 30000);
    // Scrolling by hand stops following until Follow is pressed.
    await s.exec("document.querySelector('[data-testid=viewer]').dispatchEvent(new WheelEvent('wheel', { deltaY: 200, bubbles: true }))");
    await s.waitFor('[data-testid="read-follow"]');
    await s.click('[data-testid="read-follow"]');
    await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('[data-testid=read-follow]')")), "following again");
    // Stop from the toolbar button.
    await s.click('[data-command="read.stop"]');
    await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('[data-testid=readbar]')")), "bar closed");
    assert(!(await s.exec("return !!document.querySelector('.reading-mark')")), "highlight removed");
    // Read from a page's context menu, then stop with the palette command.
    await s.exec("const p = document.querySelector('[data-page=\"1\"]'); p.scrollIntoView(); const r = p.getBoundingClientRect(); p.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: r.left + 40, clientY: r.top + 40 }))");
    await s.click('[role="menuitem"][data-command="read.aloud"]');
    // Scrolling by hand before anything is highlighted also stops following (seen on Windows, where
    // the first word is reported a moment after playback starts).
    await s.waitFor('[data-testid="readbar"]');
    await s.exec("document.querySelector('[data-testid=viewer]').dispatchEvent(new WheelEvent('wheel', { deltaY: 100, bubbles: true }))");
    await s.waitFor('[data-testid="read-follow"]');
    await s.click('[data-testid="read-follow"]');
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[data-testid=readbar]')?.dataset.status")) === "playing", "reading page 2", 30000);
    assert((await s.text('[data-testid="readbar"] [role="status"]')).includes("page 2"), "reads page 2");
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Stop reading");
    await s.click('[role="option"][data-command="read.stop"]');
    await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('[data-testid=readbar]')")), "stopped from the palette");
    // Save page 2 as audio with the saved speed.
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Save as audio");
    await s.click('[role="option"][data-command="read.saveAudio"]');
    await s.waitFor("#output");
    assert((await s.exec("return document.querySelector('#output').value")).endsWith("article.wav"), "default name");
    await s.type("#output", out);
    await s.exec("const c = [...document.querySelectorAll('[role=dialog] input[type=checkbox]')].find((x) => x.checked); if (c) c.click();");
    await s.type("#pages", "2");
    await s.waitUntil(async () => (await s.text("#pages-msg")) === "1 page selected", "page 2 chosen");
    await axeCheck(s, "save-audio-dialog");
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved article-page2.wav")), "saved notice", 120000);
  });
  const wav = readFileSync(out);
  assert(wav.subarray(0, 4).toString() === "RIFF" && wav.subarray(8, 12).toString() === "WAVE", "a WAV file");
  assert(wav.length > 44 + 16000, "with audio");
  // The speed is kept across a restart.
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.click('[data-command="read.aloud"]');
    await s.waitFor('[data-testid="readbar"]');
    assert((await s.exec("return document.querySelector('#read-rate').value")) === "1.5", "speed restored");
    await s.click('[data-testid="read-stop"]');
  });
});

await test("export tables: Excel workbook with values kept and merged cells; CSV files into a folder; nothing found is said", async () => {
  const src = join(FIX, "tables.pdf");
  const out = join(work, "tables-export.xlsx");
  const folder = mkdtempSync(join(work, "csv-"));
  const open = async (s) => {
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Export tables");
    await s.click('[role="option"][data-command="file.exportTables"]');
    await s.waitFor("#output");
  };
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await open(s);
    assert((await s.exec("return document.querySelector('#output').value")).endsWith("tables.xlsx"), "default workbook name");
    await s.click('[data-testid="sheet-format-csv"]');
    assert(!(await s.exec("return document.querySelector('#output').value")).endsWith(".xlsx"), "CSV saves into a folder");
    await s.click('[data-testid="sheet-format-xlsx"]');
    await s.type("#output", out);
    await axeCheck(s, "export-tables-dialog");
    writeFileSync(join(ART, "export-tables.png"), await s.screenshot());
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved tables-export.xlsx") && t.includes("2 tables")), "saved notice with the table count");
    await open(s);
    await s.click('[data-testid="sheet-format-csv"]');
    await s.type("#output", folder);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved 2 files")), "CSV files saved");
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("CSV keeps only the text")), "CSV loss explained");
  });
  const sheet = zipPart(out, "xl/worksheets/sheet1.xml");
  assert(sheet.includes('<mergeCell ref="A1:B1"/>'), "merged header");
  assert(sheet.includes(">007</t>") && sheet.includes(">00042</t>") && sheet.includes(">1,234.50</t>") && sheet.includes(">12.40</t>"), "values kept as text");
  assert(sheet.includes("<v>-4.5</v>"), "plain number typed");
  const csv = readFileSync(join(folder, "tables_p1_t1.csv"), "utf8");
  assert(csv.includes('007,Savings,2026-03-12,"1,234.50"'), `CSV row: ${csv}`);
  // A document without tables: the dialog says so and writes nothing.
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await open(s);
    const none = join(work, "none.xlsx");
    await s.type("#output", none);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await s.text('[role="dialog"]')).includes("No tables were found"), "reason shown in the dialog");
    assert(!existsSync(none), "nothing written");
  });
});

await test("reduce file size: tab menu and palette; smaller verified copy with the same text; sizes reported; second pass refused", async () => {
  const src = join(FIX, "uncompressed-report.pdf");
  const before = sha(src);
  const out = join(work, "report-smaller.pdf");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Reduce file size");
    await s.click('[role="option"][data-command="file.reduceSize"]');
    await s.waitFor("#output");
    assert((await s.exec("return document.querySelector('#output').value")).endsWith("uncompressed-report-smaller.pdf"), "default name");
    assert((await s.exec("return !!document.querySelector('#pages')")) === false, "whole document, no page range");
    await s.type("#output", out);
    await axeCheck(s, "reduce-size-dialog");
    writeFileSync(join(ART, "reduce-size-dialog.png"), await s.screenshot());
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved report-smaller.pdf") && /% smaller/.test(t)), "saved notice with sizes");
  });
  assert(sha(src) === before, "source untouched");
  assert(statSync(out).size * 3 < statSync(src).size, "at least two thirds smaller");
  if (CLI) {
    const t = JSON.parse(execFileSync(CLI, ["--json", "text", "--pages", "4", out], { encoding: "utf8" }));
    assert(JSON.stringify(t.result.data.pages[0].text).includes("PWREPORT4L40"), "last line of the last page is in the copy");
  }
  // Second pass on the result, from the tab menu: nothing left to gain is explained, nothing written.
  const again = join(work, "report-again.pdf");
  await withApp([out], async (s) => {
    await pageLoaded(s, 0);
    await s.contextClick(".tab-label[aria-current=page]");
    await s.click('[role="menuitem"][data-command="file.reduceSize"]');
    await s.waitFor("#output");
    await s.type("#output", again);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => ((await s.exec("return document.querySelector('[role=dialog] [role=alert]')?.textContent ?? null")) ?? "").includes("already compact"), "explained");
  });
  assert(!existsSync(again), "nothing written");
});

await test("reduce file size with image resolution: preset applied; images resampled; text kept", async () => {
  const src = join(FIX, "photo-heavy.pdf");
  const out = join(work, "photo-smaller.pdf");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Reduce file size");
    await s.click('[role="option"][data-command="file.reduceSize"]');
    await s.waitFor('[data-testid="optimize-images"]');
    assert((await s.exec("return !!document.querySelector('#optimize-preset')")) === false, "preset hidden while off");
    await s.click('[data-testid="optimize-images"]');
    await s.waitFor("#optimize-preset");
    assert((await s.exec("return document.querySelector('#optimize-preset').value")) === "standard", "standard by default");
    await s.click('#optimize-preset option[value="screen"]');
    await s.type("#output", out);
    await axeCheck(s, "reduce-size-images-dialog");
    writeFileSync(join(ART, "reduce-size-images-dialog.png"), await s.screenshot());
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved photo-smaller.pdf") && /% smaller/.test(t)), "saved notice");
  });
  assert(statSync(out).size * 3 < statSync(src).size, `at least two thirds smaller (${statSync(src).size} -> ${statSync(out).size})`);
  if (CLI) {
    const t = JSON.parse(execFileSync(CLI, ["--json", "text", out], { encoding: "utf8" }));
    const all = t.result.data.pages.map((p) => p.text).join(" ");
    for (const m of ["PWPHOTO1", "PWPHOTO2", "PWPHOTO3", "PWPHOTO4"]) assert(all.includes(m), `${m} kept`);
  }
});

await test("compare documents: with an open tab; difference list, side-by-side highlights, keyboard and buttons", async () => {
  await withApp([join(FIX, "compare-old.pdf"), join(FIX, "compare-new.pdf")], async (s) => {
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('.tab-label .name').length")) === 2, "both open");
    await s.click(".tab-label[data-doc]:first-of-type");
    await s.waitUntil(async () => (await s.exec("return document.querySelector('.tab-label[aria-current=page] .name')?.textContent ?? null")) === "compare-old.pdf", "old active");
    await pageLoaded(s, 0);
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Compare with");
    await s.click('[role="option"][data-command="file.compare"]');
    await s.waitFor('[data-testid="compare-start"]');
    assert((await s.text('[data-testid="compare-order"]')).includes("Older: compare-old.pdf · Newer: compare-new.pdf"), "order shown");
    await axeCheck(s, "compare-choose-dialog");
    await s.click('[data-testid="compare-start"]');
    await s.waitFor('[data-testid="compare-summary"]');
    assert((await s.text('[data-testid="compare-summary"]')) === "4 pages changed · 1 page inserted · 1 page deleted", `summary: ${await s.text('[data-testid="compare-summary"]')}`);
    const kinds = await s.exec("return [...document.querySelectorAll('.compare-item .compare-kind')].map(k => k.textContent)");
    assert(JSON.stringify(kinds) === JSON.stringify(["Replaced", "Inserted", "Deleted", "Format", "Page deleted", "Page inserted", "Moved", "Moved", "Visual", "Page size"]), `items: ${kinds}`);
    assert((await s.exec("return document.querySelector('.compare-meta')?.textContent ?? null")).includes("Report v1"), "title change listed");
    // First item selected: both panes show page 2 with the replaced word highlighted on each side.
    await s.waitUntil(
      async () => (await s.exec("return [...document.querySelectorAll('.compare-pane img')].filter(i => i.complete && i.naturalWidth > 0).length")) === 2,
      "two page images loaded",
    );
    assert((await s.exec("return [...document.querySelectorAll('.compare-pane-title')].map(t => t.textContent).join('|')")) === "Old · page 2|New · page 2", "page 2 on both sides");
    assert((await s.exec("return document.querySelectorAll('.compare-box.current').length")) === 2, "current change boxed on both sides");
    writeFileSync(join(ART, "compare-page2.png"), await s.screenshot());
    // Keyboard: End jumps to the last difference (page size, page 6); Up goes back one.
    await s.click(".compare-item[data-item='0']");
    await s.keys(["End"]);
    await s.waitUntil(async () => (await s.exec("return document.querySelector('.compare-item[aria-selected=true] .compare-kind')?.textContent ?? null")) === "Page size", "End selects the last item");
    await s.click('[aria-label="Previous difference"]');
    assert((await s.exec("return document.querySelector('.compare-item[aria-selected=true] .compare-kind').textContent")) === "Visual", "previous button");
    // The visual change on page 5 is drawn as a box near where the shape was drawn (lower middle).
    const box = await s.exec("const b = document.querySelectorAll('.compare-pane')[1].querySelector('.compare-box.current'); return b ? [parseFloat(b.style.left), parseFloat(b.style.top)] : null");
    assert(box && box[0] > 40 && box[0] < 55 && box[1] > 40 && box[1] < 70, `visual box position ${box}`);
    // An inserted page: the old side says so.
    await s.click(".compare-item.kind-page_inserted");
    await s.waitUntil(async () => ((await s.exec("return document.querySelector('.compare-missing')?.textContent ?? null")) ?? "") === "Not in the old document", "missing side explained");
    await axeCheck(s, "compare-result");
    writeFileSync(join(ART, "compare-result.png"), await s.screenshot());
    await s.keys(["Escape"]);
    await s.waitUntil(async () => (await s.exec("return !!document.querySelector('[role=dialog]')")) === false, "closed");
    assert((await s.exec("return document.querySelectorAll('.tab-label .name').length")) === 2, "tabs unchanged");
  });
});

await test("compare documents: another file by path; errors explained; swap order", async () => {
  await withApp([join(FIX, "compare-new.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Compare with");
    await s.click('[role="option"][data-command="file.compare"]');
    await s.waitFor("#compare-path");
    assert((await s.attr('[data-testid="compare-start"]', "title")) === "Choose the file to compare with.", "disabled with a reason");
    const alertText = async () => (await s.exec("return document.querySelector('[role=dialog] [role=alert]')?.textContent ?? null")) ?? "";
    await s.type("#compare-path", join(FIX, "not-a-pdf.pdf"));
    await s.click('[data-testid="compare-start"]');
    await s.waitUntil(async () => (await alertText()).length > 0, "error shown");
    await s.type("#compare-path", join(FIX, "encrypted-rc4-user-password.pdf"));
    await s.click('[data-testid="compare-start"]');
    await s.waitUntil(async () => /password/i.test(await alertText()), "password explained");
    // The open document is the newer version here: swap, then compare.
    await s.type("#compare-path", join(FIX, "compare-old.pdf"));
    await s.click('[data-testid="compare-swap"]');
    await s.waitUntil(async () => (await s.text('[data-testid="compare-order"]')).includes("Older: compare-old.pdf · Newer: compare-new.pdf"), "swapped");
    await s.click('[data-testid="compare-start"]');
    await s.waitFor('[data-testid="compare-summary"]');
    assert((await s.text('[data-testid="compare-summary"]')) === "4 pages changed · 1 page inserted · 1 page deleted", "same result after swapping");
    await s.keys(["Escape"]);
    await s.waitUntil(async () => (await s.exec("return !!document.querySelector('[role=dialog]')")) === false, "closed");
    assert((await s.exec("return document.querySelectorAll('.tab-label .name').length")) === 1, "the compared file did not become a tab");
  });
});

await test("signatures: status bar summary and panel for valid, tampered, edited and unsigned files; unknown signers not trusted", async () => {
  const files = ["signed-rsa.pdf", "signed-tampered.pdf", "signed-then-edited.pdf", "five-pages.pdf"];
  await withApp(files.map((f) => join(FIX, f)), async (s) => {
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('.tab-label .name').length")) === 4, "all open");
    const summaryOf = async (name) => {
      await s.exec(`[...document.querySelectorAll('.tab-label')].find(t => t.textContent.includes(${JSON.stringify(name)})).click()`);
      await s.waitUntil(async () => (await s.exec("return document.querySelector('.tab-label[aria-current=page] .name')?.textContent ?? null")) === name, `${name} active`);
      if (name === "five-pages.pdf") {
        await pageLoaded(s, 0);
        return (await s.exec("return document.querySelector('[data-testid=signature-summary]')?.textContent ?? null"));
      }
      await s.waitFor('[data-testid="signature-summary"]');
      return s.text('[data-testid="signature-summary"]');
    };
    assert((await summaryOf("signed-rsa.pdf")) === "Signed, signer not trusted", "valid signature, unknown signer");
    assert((await summaryOf("signed-tampered.pdf")) === "Signature invalid", "tampered");
    assert((await summaryOf("signed-then-edited.pdf")) === "Signed, changed since", "edited after signing");
    assert((await summaryOf("five-pages.pdf")) === null, "unsigned: no indicator");

    // Panel details for the tampered file, opened from the status bar.
    await summaryOf("signed-tampered.pdf");
    await s.click('[data-testid="signature-summary"]');
    await s.waitFor('#sb-tab-signatures[aria-selected="true"]');
    await s.waitFor('[data-signature="0"]');
    assert((await s.text('[data-signature="0"] [data-testid="signature-status"]')) === "Invalid", "panel status");
    const panel = await s.exec("return document.querySelector('.signatures').textContent");
    assert(panel.includes("PageWeave Test Signer") && panel.includes("changed after it was signed"), "signer and reason shown");
    assert(panel.includes("Revocation") && panel.includes("not checked"), "what is not checked is said");
    await axeCheck(s, "signatures-panel");
    writeFileSync(join(ART, "signatures-panel.png"), await s.screenshot());

    // Valid file: status and the field's page link.
    await summaryOf("signed-rsa.pdf");
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[data-signature=\"0\"] [data-testid=signature-status]')?.textContent ?? null")) === "Valid, signer unknown", "valid in panel, signer unknown");
    await s.exec("[...document.querySelectorAll('.signature .btn.link')].find(b => b.textContent === 'Go to page 1').click()");

    // Unsigned file via the palette command.
    await summaryOf("five-pages.pdf");
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Show signatures");
    await s.click('[role="option"][data-command="view.showSignatures"]');
    await s.waitUntil(async () => ((await s.exec("return document.querySelector('.signatures')?.textContent ?? null")) ?? "").includes("not digitally signed"), "unsigned explained");
  });
});

await test("print: Ctrl+P, page range and resolution; prepared pages at page size handed to the print UI; cleaned up afterwards", async () => {
  await withApp([join(FIX, "mixed-sizes-rotated.pdf")], async (s) => {
    await pageLoaded(s, 0);
    // The platform print UI cannot be driven here: record what would be printed instead.
    await s.exec(`window.__prints = [];
      window.print = () => {
        const root = document.querySelector('#print-root');
        window.__prints.push({
          sheets: [...root.querySelectorAll('.print-sheet')].map(d => [d.style.width, d.style.height, d.style.getPropertyValue('page')]),
          images: [...root.querySelectorAll('img')].map(i => [i.complete, i.naturalWidth, i.naturalHeight]),
          css: document.querySelector('#print-css').textContent,
        });
      }; return 1`);
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "p"]);
    await s.waitFor('[data-testid="print-start"]');
    await s.click('[data-testid="print-range"]');
    await s.type("#print-pages", "2-3");
    await s.waitUntil(async () => (await s.text("#print-pages-msg")) === "2 pages selected", "range validated");
    await s.click('#print-quality option[value="standard"]');
    await axeCheck(s, "print-dialog");
    await s.click('[data-testid="print-start"]');
    await s.waitUntil(async () => (await s.exec("return window.__prints.length")) === 1, "print UI opened once");
    const job = await s.exec("return window.__prints[0]");
    // Where the driver supports W3C "Print Page", render with print media while the pages are
    // prepared: exactly the prepared sheets print (one PDF page each), none of the app's screens.
    try {
      const pdf = Buffer.from(await s.cmd("POST", "/print", { background: true }), "base64");
      const out = join(ART, "print-media.pdf");
      writeFileSync(out, pdf);
      if (CLI) {
        const info = JSON.parse(execFileSync(CLI, ["--json", "inspect", out], { encoding: "utf8" })).result.data;
        assert(info.page_count === 2, `print media renders the 2 prepared pages (got ${info.page_count})`);
      }
    } catch (e) {
      console.log(`  note: WebDriver print-to-PDF unavailable here (${String(e.message ?? e).slice(0, 100)}); sheet layout checked from the DOM only`);
    }
    await s.exec("window.dispatchEvent(new Event('afterprint')); return 1");
    // Page 2 is A4; page 3 is Letter turned 90° (printed landscape, as displayed).
    assert(JSON.stringify(job.sheets.map((x) => [x[0], x[1]])) === JSON.stringify([["595pt", "842pt"], ["792pt", "612pt"]]), `sheet sizes ${JSON.stringify(job.sheets)}`);
    assert(job.images.every((i) => i[0] && i[1] > 0), "every page image loaded before printing");
    assert(Math.abs(job.images[0][1] - Math.round((595 * 150) / 72)) <= 2, `A4 at 150 dpi is ~1240 px wide (${job.images[0][1]})`);
    assert(job.css.includes("size: 595pt 842pt") && job.css.includes("size: 792pt 612pt"), "page sizes in @page rules");
    await s.waitUntil(async () => (await s.exec("return !!document.querySelector('[role=dialog]')")) === false, "dialog closes after printing");
    assert((await s.exec("return !!document.querySelector('#print-root')")) === false, "prepared pages removed");
    // Ctrl+P inside a text field never reaches the browser's own print (which would print the
    // app's screens): the key's default action is prevented. (Listener added after the app's.)
    await s.exec("window.addEventListener('keydown', (e) => { if (e.ctrlKey && e.key.toLowerCase() === 'p') window.__ctrlPPrevented = e.defaultPrevented; }); return 1");
    await s.keys(["Control", "f"]);
    await s.waitFor(".searchbar input");
    await s.keys(["Control", "p"]);
    assert((await s.exec("return window.__ctrlPPrevented")) === true, "browser print prevented in a text field");
  });
});

await test("straighten pages: Pages menu; skewed scans turned straight, text page left; verified by measuring the copy", async () => {
  const out = join(work, "straightened.pdf");
  await withApp([join(FIX, "skewed-scan.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Straighten");
    await s.click('[role="option"][data-command="pages.deskew"]');
    await s.waitFor("#output");
    assert((await s.exec("return document.querySelector('#output').value")).endsWith("skewed-scan-straightened.pdf"), "default name");
    await s.type("#output", out);
    await axeCheck(s, "deskew-dialog");
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved straightened.pdf") && t.includes("3 pages straightened")), "saved notice");
  });
  if (CLI) {
    // Measuring the copy again finds nothing left to straighten.
    const again = spawnSync(CLI, ["--json", "deskew", "-o", join(work, "again.pdf"), out], { encoding: "utf8" });
    assert(JSON.parse(again.stdout).error.code === "deskew.nothing", `copy is straight: ${again.stdout.slice(0, 200)}`);
  }
  // Nothing to do: the dialog explains and writes nothing.
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Straighten");
    await s.click('[role="option"][data-command="pages.deskew"]');
    await s.waitFor("#output");
    await s.type("#output", join(work, "never.pdf"));
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => ((await s.exec("return document.querySelector('[role=dialog] [role=alert]')?.textContent ?? null")) ?? "").includes("No page needed straightening"), "explained");
  });
  assert(!existsSync(join(work, "never.pdf")), "nothing written");
});

await test("remove blank pages: found with thumbnails; sensitivity; untick one; removed into a new file", async () => {
  const out = join(work, "no-blank.pdf");
  await withApp([join(FIX, "blank-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Remove blank");
    await s.click('[role="option"][data-command="pages.removeBlank"]');
    // Standard: the empty page (2) and the dusty scan (3); the page number page and faint table stay.
    await s.waitUntil(async () => (await s.exec("return [...document.querySelectorAll('[data-blank-page]')].map(c => c.dataset.blankPage).join(',')")) === "1,2", "pages 2 and 3 found");
    assert((await s.text('[data-testid="blank-remove"]')) === "Remove 2 pages", "count on the button");
    // Only truly empty pages: the dusty scan is no longer listed.
    await s.click('#blank-preset option[value="empty"]');
    await s.waitUntil(async () => (await s.exec("return [...document.querySelectorAll('[data-blank-page]')].map(c => c.dataset.blankPage).join(',')")) === "1", "only page 2");
    await s.click('#blank-preset option[value="standard"]');
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('[data-blank-page]').length")) === 2, "back to two");
    await s.click('[data-blank-page="2"]'); // keep the dusty scan after all
    assert((await s.text('[data-testid="blank-remove"]')) === "Remove 1 page", "unticked");
    await s.type("#blank-output", out);
    await axeCheck(s, "remove-blank-dialog");
    writeFileSync(join(ART, "remove-blank-dialog.png"), await s.screenshot());
    await s.click('[data-testid="blank-remove"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved no-blank.pdf")), "saved notice");
  });
  if (CLI) {
    const info = JSON.parse(execFileSync(CLI, ["--json", "inspect", out], { encoding: "utf8" })).result.data;
    assert(info.page_count === 5, `one page removed (${info.page_count})`);
    const t = JSON.parse(execFileSync(CLI, ["--json", "text", out], { encoding: "utf8" })).result.data.pages.map((p) => p.text).join(" ");
    assert(t.includes("PWBLANK1") && t.includes("PWBLANK6"), "text pages kept");
  }
  // A document without blank pages says so and offers nothing to remove.
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Remove blank");
    await s.click('[role="option"][data-command="pages.removeBlank"]');
    await s.waitFor('[data-testid="blank-none"]');
    assert((await s.attr('[data-testid="blank-remove"]', "title")) === "No blank pages found.", "disabled with a reason");
  });
});

await test("extract images: Pages menu; JPEG saved exactly, others as PNG, repeats once; none found explained", async () => {
  const dir = mkdtempSync(join(work, "extract-"));
  await withApp([join(FIX, "photo-heavy.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Extract images");
    await s.click('[role="option"][data-command="pages.extractImages"]');
    await s.waitFor("#output");
    await s.type("#output", dir);
    await axeCheck(s, "extract-images-dialog");
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved 4 files")), "saved notice");
  });
  const files = readdirSync(dir).sort();
  assert(JSON.stringify(files) === JSON.stringify(["photo-heavy_p001_01.jpg", "photo-heavy_p002_01.png", "photo-heavy_p003_01.png", "photo-heavy_p003_02.png"]), `files: ${files}`);
  assert(readFileSync(join(dir, files[0])).subarray(0, 2).toString("hex") === "ffd8", "a JPEG file");
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Extract images");
    await s.click('[role="option"][data-command="pages.extractImages"]');
    await s.waitFor("#output");
    await s.type("#output", dir);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => ((await s.exec("return document.querySelector('[role=dialog] [role=alert]')?.textContent ?? null")) ?? "").includes("no embedded images"), "explained");
  });
});

await test("turn pages upright: Pages menu; sideways and upside-down scans turned; verified rotations", async () => {
  const out = join(work, "upright.pdf");
  await withApp([join(FIX, "turned-scan.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Turn pages upright");
    await s.click('[role="option"][data-command="pages.orient"]');
    await s.waitFor("#output");
    assert((await s.exec("return document.querySelector('#output').value")).endsWith("turned-scan-upright.pdf"), "default name");
    // Two pages keep the scenario short (each page is read four times).
    await s.click('[role="dialog"] input[type="checkbox"]'); // untick "All pages"
    await s.type("#pages", "1-2");
    await s.type("#output", out);
    await axeCheck(s, "orient-dialog");
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved upright.pdf") && t.includes("2 pages turned upright")), "saved notice", 180000);
  });
  if (CLI) {
    const info = JSON.parse(execFileSync(CLI, ["--json", "inspect", out], { encoding: "utf8" })).result.data;
    const rot = info.pages.map((p) => p.rotation);
    assert(JSON.stringify(rot.slice(0, 3)) === "[90,180,0]", `rotations ${JSON.stringify(rot)}`);
  }
});

await test("sign with a certificate: wrong password explained; visible signature placed as previewed; verified valid", async () => {
  const out = join(work, "signed.pdf");
  const cert = join(FIX, "..", "keys", "test-signer-rsa.p12");
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Sign with a certificate");
    await s.click('[role="option"][data-command="file.sign"]');
    await s.waitFor("#sign-cert");
    assert((await s.attr('[data-testid="sign-start"]', "title")) === "Choose your certificate file (.pfx or .p12).", "blocked with a reason");
    await s.type("#sign-cert", cert);
    await s.type("#sign-pw", "wrong-password");
    await s.type("#sign-reason", "Approved");
    await s.type("#sign-output", out);
    await s.waitFor('[data-testid="placement-box"]');
    await axeCheck(s, "sign-dialog");
    writeFileSync(join(ART, "sign-dialog.png"), await s.screenshot());
    await s.click('[data-testid="sign-start"]');
    await s.waitUntil(async () => ((await s.exec("return document.querySelector('[role=dialog] [role=alert]')?.textContent ?? null")) ?? "").includes("password is not correct"), "wrong password explained");
    assert(!existsSync(out), "nothing written");
    await s.exec("const i = document.querySelector('#sign-pw'); const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set; set.call(i, ''); i.dispatchEvent(new Event('input', { bubbles: true })); return 1");
    await s.type("#sign-pw", "pageweave-test");
    await s.click('[data-testid="sign-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved signed.pdf") && t.includes("Signed as PageWeave Test Signer")), "saved notice");
  });
  if (CLI) {
    const v = JSON.parse(execFileSync(CLI, ["--json", "signatures", out], { encoding: "utf8" })).result.data.signatures;
    assert(v.length === 1 && v[0].status === "valid" && v[0].covers_whole_file && v[0].page === 0 && v[0].reason === "Approved", `verified: ${JSON.stringify(v).slice(0, 300)}`);
  }
});

await test("signature or initials: typed and drawn (pointer) placed as previewed; labelled as not a digital signature", async () => {
  const typedOut = join(work, "typed-sig.pdf");
  const drawnOut = join(work, "drawn-sig.pdf");
  const open = async (s) => {
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Add signature or initials");
    await s.click('[role="option"][data-command="file.signatureMark"]');
    await s.waitFor("#mark-text");
  };
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await open(s);
    assert((await s.text('[role="dialog"] .hint')).includes("not a digital signature"), "honest label");
    assert((await s.attr('[data-testid="mark-start"]', "title")) === "Type your name or initials.", "blocked with a reason");
    await s.type("#mark-text", "Ada Lovelace");
    // The preview is decorative (aria-hidden), which WebDriver reports as not displayed.
    assert((await s.exec("return document.querySelector('.typed-preview').textContent")) === "Ada Lovelace", "preview");
    await s.replaceText("#mark-output", typedOut);
    await s.waitFor('[data-testid="placement-box"]');
    await axeCheck(s, "signature-mark-dialog");
    writeFileSync(join(ART, "signature-mark-typed.png"), await s.screenshot());
    await s.click('[data-testid="mark-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved typed-sig.pdf")), "typed saved");
    // Drawn: real pointer strokes on the pad.
    await open(s);
    await s.click("#mark-tab-drawn");
    await s.waitFor('[data-testid="signature-pad"]');
    assert((await s.attr('[data-testid="mark-start"]', "title")) === "Draw your signature first.", "draw first");
    const r = await s.exec("const b = document.querySelector('[data-testid=signature-pad]').getBoundingClientRect(); return [b.left, b.top, b.width, b.height]");
    const [x, y, w, h] = r;
    await s.dragAt(x + w * 0.1, y + h * 0.6, x + w * 0.5, y + h * 0.3, 10);
    await s.dragAt(x + w * 0.5, y + h * 0.3, x + w * 0.9, y + h * 0.7, 10);
    assert((await s.attr('[data-testid="signature-pad"]', "aria-label")) === "Signature drawing, 2 strokes", "two strokes recorded");
    await s.click('[data-testid="pad-clear"]');
    assert((await s.attr('[data-testid="signature-pad"]', "aria-label")) === "Empty signature drawing area", "cleared");
    await s.dragAt(x + w * 0.1, y + h * 0.5, x + w * 0.9, y + h * 0.5, 10);
    await s.click('#mark-position option[value="top-left"]');
    await s.replaceText("#mark-output", drawnOut);
    writeFileSync(join(ART, "signature-mark-drawn.png"), await s.screenshot());
    await s.click('[data-testid="mark-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved drawn-sig.pdf")), "drawn saved");
  });
  if (CLI) {
    const t = JSON.parse(execFileSync(CLI, ["--json", "text", "--pages", "1", typedOut], { encoding: "utf8" })).result.data.pages[0].text;
    assert(t.includes("Ada Lovelace"), "typed name is text in the copy");
    assert(statSync(drawnOut).size > statSync(join(FIX, "five-pages.pdf")).size, "drawn copy written");
  }
});

await test("trust a signer: unknown → trust the CA after seeing its fingerprint → trusted; remove → unknown again", async () => {
  if (!CLI) throw new Skip("needs the CLI to sign the test file");
  const signed = join(work, "to-trust.pdf");
  const r = spawnSync(CLI, ["sign", "--certificate", join(FIX, "..", "keys", "test-signer-rsa.p12"), "-o", signed, join(FIX, "five-pages.pdf")], { input: "pageweave-test\n", encoding: "utf8" });
  assert(r.status === 0, `signed: ${r.stderr}`);
  await withApp([signed], async (s) => {
    await s.waitFor('[data-testid="signature-summary"]');
    assert((await s.text('[data-testid="signature-summary"]')) === "Signed, signer not trusted", "unknown at first");
    await s.click('[data-testid="signature-summary"]');
    await s.waitFor('[data-testid="trust-signer"]');
    await s.click('[data-testid="trust-signer"]');
    await s.waitFor('[data-trust-position="1"]');
    const dialog = await s.text('[role="dialog"]');
    assert(dialog.includes("Only PageWeave Test Signer") && dialog.includes("Everything issued by PageWeave Test CA"), "both choices offered");
    assert(/([0-9A-F]{2}:){31}[0-9A-F]{2}/.test(dialog), "fingerprints shown");
    await axeCheck(s, "trust-dialog");
    await s.click('[data-trust-position="1"]');
    await s.click('[data-testid="trust-confirm"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Trusted PageWeave Test CA")), "trusted notice");
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[data-testid=signature-summary]')?.textContent ?? null")) === "Signed", "status bar now trusted");
    assert((await s.text('[data-signature="0"] [data-testid="signature-status"]')) === "Valid, trusted", "panel trusted");
    assert((await s.exec("return document.querySelector('.signatures').textContent")).includes("PageWeave Test Signer → PageWeave Test CA"), "chain shown");
    writeFileSync(join(ART, "signatures-trusted.png"), await s.screenshot());
    // Manage from the palette, then from the panel link; remove the trust again (also leaves later
    // scenarios unaffected).
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Trusted certificates");
    await s.click('[role="option"][data-command="file.trustedCertificates"]');
    await s.waitFor('[data-trusted="PageWeave Test CA"]');
    await s.keys(["Escape"]);
    await s.exec("[...document.querySelectorAll('.signatures .btn.link')].find(b => b.textContent.startsWith('Trusted certificates')).click()");
    await s.waitFor('[data-trusted="PageWeave Test CA"]');
    await axeCheck(s, "trusted-certificates-dialog");
    await s.click('[aria-label="Remove PageWeave Test CA from trusted certificates"]');
    await s.waitFor('[data-testid="trusted-none"]');
    await s.keys(["Escape"]);
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[data-testid=signature-summary]')?.textContent ?? null")) === "Signed, signer not trusted", "unknown again");
  });
});

await test("edit text and pictures: inline text edit, drag and turn a picture, add text, undo/redo; saved copy verified", async () => {
  const out = join(work, "content-edited.pdf");
  await withApp([join(FIX, "embedded-font.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", "e"]);
    await s.waitFor('[data-testid="edit-layer"]');
    assert((await s.attr('[data-command="edit.toggleEditMode"]', "aria-pressed")) === "true", "Edit mode shown as on");
    await s.waitFor('[data-edit-text="0"]');
    assert((await s.attr('[data-edit-text="0"]', "aria-label")) === "Edit text: Invoice 2026 total due", "text run listed");
    // Text: click the run, change the wording inline, Enter commits.
    await s.click('[data-edit-text="0"]');
    await s.replaceText('[data-testid="edit-text-input"]', "Invoice 2026 total sum");
    await s.keys(["Enter"]);
    await s.waitUntil(async () => (await s.attr('[data-edit-text="0"]', "aria-label")) === "Edit text: Invoice 2026 total sum", "edit recorded");
    await s.waitFor('[data-testid="edit-preview"]');
    // Escape in the inline field cancels without recording a change.
    await s.click('[data-edit-text="2"]');
    await s.replaceText('[data-testid="edit-text-input"]', "never saved");
    await s.keys(["Escape"]);
    await s.waitUntil(async () => (await s.exec("return !document.querySelector('[data-testid=edit-text-input]')")) === true, "cancelled");
    assert((await s.exec("return !!document.querySelector('[data-testid=edit-layer]')")) === true, "Escape in the field keeps Edit mode");
    await s.waitUntil(async () => (await unsavedText(s)) === "1 unsaved change", "one change");
    // Picture: drag it to the right with the pointer, then turn it from its toolbar.
    const [x, y, w, h] = await s.exec("const b = document.querySelector('[data-edit-image=\"0\"]').getBoundingClientRect(); return [b.left, b.top, b.width, b.height]");
    await s.dragAt(x + w / 2, y + h / 2, x + w / 2 + 120, y + h / 2, 10);
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[data-edit-image=\"0\"]').getBoundingClientRect().left")) > x + 100, "box follows the move");
    await s.click('[data-edit-image="0"]');
    await s.click('[aria-label="Turn picture 90° clockwise"]');
    await s.waitUntil(async () => (await unsavedText(s)) === "3 unsaved changes", "move and turn recorded");
    // The toolbar's Delete removes the picture's box; undo brings it back.
    await s.click('[aria-label="Delete picture"]');
    await s.waitUntil(async () => (await s.exec("return !document.querySelector('[data-edit-image]')")) === true, "picture deleted");
    await s.keys(["Control", "z"]);
    await s.waitUntil(async () => (await s.exec("return !!document.querySelector('[data-edit-image]')")) === true, "delete undone");
    await s.waitUntil(async () => (await unsavedText(s)) === "3 unsaved changes", "still three changes");
    // Add text: arm the tool, click on the page, type, Enter.
    await s.click('[data-command="edit.addText"]');
    await s.click('[data-testid="add-text-capture"]');
    await s.type('[data-testid="add-text-input"]', "PWADDED note");
    await s.keys(["Enter"]);
    await s.waitUntil(async () => (await unsavedText(s)) === "4 unsaved changes", "added text recorded");
    // Undo and redo walk the content edits.
    await s.keys(["Control", "z"]);
    await s.waitUntil(async () => (await unsavedText(s)) === "3 unsaved changes", "undo");
    await s.keys(["Control", "y"]);
    await s.waitUntil(async () => (await unsavedText(s)) === "4 unsaved changes", "redo");
    await axeCheck(s, "edit-mode");
    writeFileSync(join(ART, "edit-mode.png"), await s.screenshot());
    // Escape on the page leaves Edit mode; the edits stay pending.
    await s.keys(["Escape"]);
    await s.waitUntil(async () => (await s.exec("return !document.querySelector('[data-testid=edit-layer]')")) === true, "Edit mode off");
    await s.keys(["Control", "s"]);
    await s.waitFor("#output");
    assert((await s.text('[role="dialog"] .hint')).includes("4 content edits"), "save dialog counts the content edits");
    await s.replaceText("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved content-edited.pdf")), "saved notice");
    await s.waitUntil(async () => (await unsavedText(s)) === "", "pending edits cleared after saving");
  });
  assert(existsSync(out), "edited copy exists");
  if (CLI) {
    const t = JSON.parse(execFileSync(CLI, ["--json", "text", "--pages", "1", out], { encoding: "utf8" })).result.data.pages[0].text;
    assert(t.includes("Invoice 2026 total sum") && !t.includes("total due"), `text replaced: ${t}`);
    assert(t.includes("PWADDED note"), "added text present");
    assert(t.includes("Standard font line"), "cancelled edit not applied");
    const objs = JSON.parse(execFileSync(CLI, ["--json", "objects", "--page", "1", out], { encoding: "utf8" })).result.data;
    assert(objs.images.length === 1 && objs.images[0].width_px === 80 && objs.images[0].height_px === 120, `picture turned: ${JSON.stringify(objs.images)}`);
    const before = JSON.parse(execFileSync(CLI, ["--json", "objects", "--page", "1", join(FIX, "embedded-font.pdf")], { encoding: "utf8" })).result.data;
    assert(objs.images[0].rect.left > before.images[0].rect.left + 40, "picture moved right");
  }
});

await test("edit pictures: resize by a corner and with Ctrl+arrows, then move; the saved copy has the new box", async () => {
  const src = join(FIX, "embedded-font.pdf");
  const out = join(work, "picture-resized.pdf");
  const box = (s) => s.exec("const b = document.querySelector('[data-edit-image=\"0\"]').getBoundingClientRect(); return { x: b.left, y: b.top, w: b.width, h: b.height }");
  const before = CLI ? JSON.parse(execFileSync(CLI, ["--json", "objects", "--page", "1", src], { encoding: "utf8" })).result.data.images[0].rect : null;
  let k = 1;
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", "e"]);
    await s.waitFor('[data-edit-image="0"]');
    await s.click('[data-edit-image="0"]');
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('.edit-handle').length")) === 4, "four corner handles on the selected picture");
    const b0 = await box(s);
    if (before) k = b0.w / (before.right - before.left);
    // Bottom-right corner 60 px right and 30 px down; the top-left corner stays.
    const se = await s.exec("const r = document.querySelector('.edit-handle[data-corner=se]').getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]");
    await s.dragAt(se[0], se[1], se[0] + 60, se[1] + 30);
    await s.waitUntil(async () => { const b = await box(s); return Math.abs(b.w - (b0.w + 60)) < 1.5 && Math.abs(b.h - (b0.h + 30)) < 1.5 && Math.abs(b.x - b0.x) < 1; }, "corner drag resized the box");
    await s.waitUntil(async () => (await unsavedText(s)) === "1 unsaved change", "resize recorded");
    // Ctrl+Shift+Right: 10 pt wider (the picture keeps the focus).
    await s.click('[data-edit-image="0"]');
    await s.keys(["Control", "Shift", "ArrowRight"]);
    await s.waitUntil(async () => Math.abs((await box(s)).w - (b0.w + 60 + 10 * k)) < 1.5, "10 pt wider from the keyboard");
    // A move after the resize still applies.
    const b1 = await box(s);
    await s.dragAt(b1.x + b1.w / 2, b1.y + b1.h / 2, b1.x + b1.w / 2 + 50, b1.y + b1.h / 2, 10);
    await s.waitUntil(async () => Math.abs((await box(s)).x - (b1.x + 50)) < 1.5 && Math.abs((await box(s)).w - b1.w) < 1, "moved, size kept");
    await s.waitFor('[data-testid="edit-preview"]');
    await axeCheck(s, "edit-picture-resize");
    writeFileSync(join(ART, "edit-picture-resized.png"), await s.screenshot());
    await s.keys(["Escape"]);
    await s.keys(["Control", "s"]);
    await s.waitFor("#output");
    await s.replaceText("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved picture-resized.pdf")), "saved notice");
  });
  if (CLI) {
    const r = JSON.parse(execFileSync(CLI, ["--json", "objects", "--page", "1", out], { encoding: "utf8" })).result.data.images[0].rect;
    const near = (a, b) => Math.abs(a - b) < 0.8;
    const ok = near(r.left, before.left + 50 / k) && near(r.top, before.top) && near(r.right - r.left, before.right - before.left + 60 / k + 10) && near(r.top - r.bottom, before.top - before.bottom + 30 / k);
    assert(ok, `picture ${JSON.stringify(before)} → ${JSON.stringify(r)} (k ${k})`);
  }
});

await test("combine: open files added with one button, reordered, combined; first file's title and every page kept", async () => {
  const files = [join(FIX, "single-page.pdf"), join(FIX, "article.pdf")];
  const out = join(work, "combined-open.pdf");
  const names = (s) => s.exec("return [...document.querySelectorAll('[aria-label=\"Files to combine\"] li .name')].map(n => n.textContent.replace(/^\\d+\\. /, ''))");
  let movedFirst = "";
  await withApp(files, async (s) => {
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('.tab-label').length")) === 2, "two tabs");
    await pageLoaded(s, 0);
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "Shift", "M"]);
    await s.waitFor('[data-testid="merge-add-open"]');
    const first = (await names(s))[0];
    assert((await s.text('[data-testid="merge-add-open"]')).includes("(1)"), "one other open file offered");
    await s.click('[data-testid="merge-add-open"]');
    await s.waitUntil(async () => (await names(s)).length === 2, "the open file added");
    assert((await s.attr('[data-testid="merge-add-open"]', "aria-disabled")) === "true", "nothing more to add");
    // Put the other file first.
    const second = (await names(s))[1];
    movedFirst = second;
    await s.click(`[aria-label="Move ${second} up"]`);
    await s.waitUntil(async () => (await names(s))[0] === second && (await names(s))[1] === first, "reordered");
    await s.replaceText("#output", out);
    await axeCheck(s, "combine-dialog");
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved combined-open.pdf")), "saved notice");
  });
  if (CLI) {
    const info = (f) => JSON.parse(execFileSync(CLI, ["--json", "inspect", f], { encoding: "utf8" })).result.data;
    const pageText = (f) => JSON.parse(execFileSync(CLI, ["--json", "text", "--pages", "1", f], { encoding: "utf8" })).result.data.pages[0].text.trim();
    const firstFile = join(FIX, movedFirst);
    const got = info(out);
    assert(got.page_count === files.reduce((n, f) => n + info(f).page_count, 0), `every page kept: ${got.page_count}`);
    assert(got.title === info(firstFile).title, `the first file's title: ${got.title}`);
    assert(pageText(out) === pageText(firstFile), "page 1 comes from the file moved to the top");
  }
});

await test("protect with password: AES-256 copy needs the password; wrong password refused; content intact", async () => {
  const src = join(FIX, "five-pages.pdf");
  const before = sha(src);
  const out = join(work, "locked.pdf");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Protect with password");
    await s.click('[role="option"][data-command="file.protect"]');
    await s.waitFor("#protect-pw");
    const reason = () => s.attr('[data-testid="protect-start"]', "title");
    assert((await reason()) === "Enter a password.", "disabled with a reason until a password is entered");
    await s.type("#protect-pw", "E2e-pw 1");
    await s.type("#protect-confirm", "E2e-pw 2");
    await s.waitUntil(async () => (await reason()) === "The passwords do not match.", "mismatch explained");
    assert((await s.attr("#protect-pw", "type")) === "password", "hidden by default");
    await s.click('button[aria-label="Show passwords"]');
    assert((await s.attr("#protect-pw", "type")) === "text", "show toggle reveals");
    await s.click('button[aria-label="Hide passwords"]');
    await s.type("#protect-confirm", "E2e-pw 1");
    await s.click('input[data-perm="print"]');
    await s.waitFor("#protect-owner");
    await s.type("#protect-output", out);
    await axeCheck(s, "protect-dialog");
    await s.waitUntil(async () => (await s.attr('[data-testid="protect-start"]', "aria-disabled")) === null, "ready");
    await s.click('[data-testid="protect-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved locked.pdf")), "saved notice", 30000);
    // Open the copy: it asks for the password; a wrong one is refused.
    await s.exec("[...document.querySelectorAll('.notice button')].find(b => b.textContent === 'Open').click(); return 1");
    await s.waitFor("#pdf-password");
    await s.type("#pdf-password", "wrong");
    await s.keys(["Enter"]);
    await s.waitUntil(async () => (await s.text("#pw-err")).length > 0, "wrong password message");
    await s.type("#pdf-password", "E2e-pw 1");
    await s.keys(["Enter"]);
    await s.waitUntil(async () => (await s.exec("return [...document.querySelectorAll('.tab-label .name')].map(n => n.textContent)")).includes("locked.pdf"), "opens with the password");
    await pageLoaded(s, 0);
  });
  assert(sha(src) === before, "source PDF unchanged");
  assert(!readFileSync(out).toString("latin1").includes("PWMARK"), "content not readable without decrypting");
  if (CLI) {
    const locked = spawnSync(CLI, ["inspect", out]);
    assert(locked.status === 5, `CLI needs the password (exit ${locked.status})`);
    const r = spawnSync(CLI, ["--json", "--password-stdin", "text", "--pages", "4", out], { input: "E2e-pw 1\n", encoding: "utf8" });
    assert(r.status === 0 && JSON.parse(r.stdout).result.data.pages[0].text.includes("PWMARK004"), `independent text check: ${r.stdout.slice(0, 200)}`);
  }
});

/** A pointer drag with Alt held throughout. Alt is pressed in its own action call and stays down
 *  (WebDriver keeps input state between calls) until the release: when key and pointer actions
 *  share one call, their order within a tick is up to the driver, and the last move sometimes went
 *  out without Alt (snapping then moved the object 2.5 px). */
async function altDrag(s, x0, y0, x1, y1) {
  await s.cmd("POST", "/actions", { actions: [{ type: "key", id: "kb", actions: [{ type: "keyDown", value: "\uE00A" }] }] });
  await s.cmd("POST", "/actions", { actions: [
    { type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
      { type: "pointerMove", origin: "viewport", x: Math.round(x0), y: Math.round(y0) },
      { type: "pointerDown", button: 0 },
      { type: "pointerMove", origin: "viewport", duration: 100, x: Math.round(x1), y: Math.round(y1) },
      { type: "pointerUp", button: 0 },
    ] },
  ] });
  await s.cmd("POST", "/actions", { actions: [{ type: "key", id: "kb", actions: [{ type: "keyUp", value: "\uE00A" }] }] });
  await s.cmd("DELETE", "/actions");
}

/** Ctrl+S in the Studio, then wait until the project on disk has changed and the status says so
 *  (a status left over from an earlier save must not count). */
async function studioSave(s, file) {
  const before = readFileSync(file, "utf8");
  const size = () => s.exec("const r=document.querySelector('[data-testid=studio-stage]').getBoundingClientRect(); return [r.width, r.height]");
  const [w0, h0] = await size();
  await s.keys(["Control", "s"]);
  await s.waitUntil(async () => readFileSync(file, "utf8") !== before && (await s.text('[data-testid="studio-status"]')).startsWith("Saved"), "saved");
  // The "Saved" message must not re-fit the page (it once added a row and shrank the canvas).
  await new Promise((r) => setTimeout(r, 100));
  const [w1, h1] = await size();
  assert(Math.abs(w1 - w0) < 0.5 && Math.abs(h1 - h0) < 0.5, `page size unchanged by the save message (${w0}×${h0} → ${w1}×${h1})`);
}

await test("design studio: rebuild a page, edit text with undo, nudge, drag, lock, hide, compare, save; the project reopens", async () => {
  const src = join(work, "studio-poster.pdf");
  copyFileSync(join(FIX, "poster.pdf"), src);
  const project = join(work, "studio-poster-page1.pwdesign");
  const objX = (s) => s.exec("return Number(document.querySelector('.studio-grid input').value)");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    // Disabled reasons are explained elsewhere; here it is available with a document.
    assert((await s.attr('.toolbar [data-command="studio.rebuildPage"]', "aria-disabled")) === null, "rebuild available");
    await s.click('.toolbar [data-command="studio.rebuildPage"]');
    await s.waitFor('[data-testid="studio"]');
    await s.waitFor('[data-testid="studio-layers"] [data-layer]', { timeout: 60000 });
    const layers = await s.exec("return [...document.querySelectorAll('[data-layer]')].map(b => b.dataset.layer)");
    assert(layers.length === 9 && layers.includes("picture-1"), `layers ${layers}`);
    assert((await s.text('[data-testid="studio-summary"]')).includes("5 text · 3 shapes · 1 pictures"), "summary counts");
    // The two body lines are one paragraph, drawn line by line; it can be centred (and undone).
    const para = await s.exec("const t=[...document.querySelectorAll('[data-testid=studio-stage] text')].find(t => t.textContent.startsWith('Music')); return t ? [t.dataset.object, [...t.querySelectorAll('tspan')].map(x => x.textContent)] : null");
    assert(para && para[1].join("|") === "Music, food and crafts|Riverside Park, 10:00 to 18:00", `paragraph ${JSON.stringify(para)}`);
    await s.click(`[data-layer="${para[0]}"]`);
    await s.click('[role="radio"][aria-label="Align centre"]');
    await s.waitUntil(async () => (await s.exec(`return document.querySelector('[data-object="${para[0]}"]').getAttribute('text-anchor')`)) === "middle", "centred");
    await s.exec("document.querySelector('[data-testid=studio-stage]').focus()");
    await s.keys(["Control", "z"]);
    await s.waitUntil(async () => (await s.exec(`return document.querySelector('[data-object="${para[0]}"]').getAttribute('text-anchor')`)) === "start", "undo alignment");
    await s.keys(["Escape"]);
    assert(existsSync(project), "project saved next to the PDF");
    await axeCheck(s, "design-studio");
    writeFileSync(join(ART, "design-studio.png"), await s.screenshot());

    // Edit the title's wording; undo and redo it from the keyboard.
    const title = () => s.exec("return document.querySelector('[data-object=\"text-1\"]').textContent");
    assert((await title()) === "SPRING FAIR", `title ${await title()}`);
    await s.click('[data-layer="text-1"]');
    await s.click("#studio-text");
    await s.keys(["Control", "a"]);
    await s.type("#studio-text", "SUMMER FAIR");
    await s.waitUntil(async () => (await title()) === "SUMMER FAIR", "canvas text updated");
    await s.keys(["Control", "z"]);
    await s.waitUntil(async () => (await title()) === "SPRING FAIR", "one undo step for the whole word");
    await s.keys(["Control", "y"]);
    await s.waitUntil(async () => (await title()) === "SUMMER FAIR", "redo");

    // Keyboard nudge on the canvas (Shift = 10 px).
    const x0 = await objX(s);
    await s.exec("document.querySelector('[data-testid=studio-stage]').focus()");
    await s.keys(["Shift", "ArrowRight"]);
    await s.keys(["ArrowRight"]);
    await s.waitUntil(async () => (await objX(s)) === x0 + 11, "nudged 11 px");

    // Drag the picture with the pointer; a locked picture stays put.
    await s.click('[data-layer="picture-1"]');
    const box = () => s.exec("const r=document.querySelector('[data-object=\"picture-1\"]').getBoundingClientRect(); return [r.left, r.top, r.width, r.height]");
    const [l, t, w, h] = await box();
    await s.dragAt(l + w / 2, t + h / 2, l + w / 2 - 60, t + h / 2 + 20);
    await s.waitUntil(async () => (await box())[0] < l - 40, "dragged left");
    const [l2] = await box();
    await s.click('[aria-label="Lock Picture 1"]');
    await s.dragAt(l2 + w / 2, t + h / 2 + 20, l2 + w / 2 + 80, t + h / 2 + 20);
    assert(Math.abs((await box())[0] - l2) < 1, "locked picture not moved");

    // Hide a shape; it leaves the canvas and its row says so.
    await s.click('[data-layer="shape-1"]');
    await s.click('[aria-label="Hide Shape 1"]');
    await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('[data-object=\"shape-1\"]')")), "hidden");

    // Compare with the original: overlay with opacity, differences blend.
    await s.click('[data-compare="overlay"]');
    await s.waitFor('.studio-opacity');
    assert(Number(await s.exec("return getComputedStyle(document.querySelector('.studio-source')).opacity")) === 0.5, "overlay at 50%");
    await s.click('[data-compare="difference"]');
    assert((await s.exec("return getComputedStyle(document.querySelector('.studio-source')).mixBlendMode")) === "difference", "differences");
    await s.click('[data-compare="design"]');

    // Add text and a rectangle from the Studio toolbar; drop the rectangle 3 screen px off the canvas
    // centre: it snaps (nearest target wins), so one of its edges or its centre lands exactly on a
    // canvas or object edge/centre, read back from the drawn SVG. No guide is left after the drag.
    await s.click('[data-testid="studio-add-text"]');
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[data-layer][aria-current]')?.textContent ?? ''")).includes("New text"), "text added and selected");
    await s.click('[data-testid="studio-add-rect"]');
    const rectId = await s.exec("return document.querySelector('[data-layer][aria-current]').dataset.layer");
    assert(rectId.startsWith("shape-new"), `rectangle added and selected (${rectId})`);
    const at = (id) => s.exec(`const r=document.querySelector('[data-object="${id}"]').getBoundingClientRect(); return [r.left, r.top, r.width, r.height]`);
    const [rl, rt, rw, rh] = await at(rectId);
    const [sl, st, sw, sh] = await s.exec("const r=document.querySelector('[data-testid=studio-stage]').getBoundingClientRect(); return [r.left, r.top, r.width, r.height]");
    await s.dragAt(rl + rw / 2, rt + rh / 2, sl + sw / 2 + 3, st + sh * 0.85, 12);
    // Exact boxes from the saved project (a paragraph's drawn <text> carries no box attributes).
    await studioSave(s, project);
    const snapped = JSON.parse(readFileSync(project, "utf8"));
    const span = (o) => [o.x, o.x + o.w / 2, o.x + o.w];
    const me = span(snapped.objects.find((o) => o.id === rectId));
    const targets = [0, snapped.width / 2, snapped.width, ...snapped.objects.filter((o) => o.id !== rectId && o.visible).flatMap(span)];
    const aligned = me.some((v) => targets.some((t) => Math.abs(v - t) < 0.06));
    assert(aligned, "the dropped rectangle lines up exactly with a canvas or object edge/centre");
    assert(!(await s.exec("return !!document.querySelector('[data-testid=studio-guide]')")), "guides gone after the drag");
    // Alt places freely: a drop 3 px off is kept as dropped (not pulled onto a guide).
    const [al, at2, aw, ah] = await at(rectId);
    await altDrag(s, al + aw / 2, at2 + ah / 2, al + aw / 2 + 40, at2 + ah / 2);
    const [al2] = await at(rectId);
    assert(Math.abs(al2 - (al + 40)) < 0.75, `with Alt the rectangle moved exactly as dragged (${al2 - al} px)`);

    // Several objects: Shift+click two layers, drag one on the canvas → both move by the same amount
    // (Alt: no snapping). Then turn one 90° from its properties.
    await s.click('[data-layer="text-4"]');
    await s.exec("document.querySelector('[data-layer=\"text-5\"]').dispatchEvent(new MouseEvent('click', { bubbles: true, shiftKey: true }))");
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('[data-layer][aria-current]').length")) === 2, "two selected");
    const [a0] = await at("text-4");
    const [b0] = await at("text-5");
    const [, t4top, t4w, t4h] = await at("text-4");
    await altDrag(s, a0 + t4w / 2, t4top + t4h / 2, a0 + t4w / 2 + 30, t4top + t4h / 2);
    const [a1] = await at("text-4");
    const [b1] = await at("text-5");
    assert(Math.abs(a1 - a0 - 30) < 0.75 && Math.abs(b1 - b0 - 30) < 0.75, `both moved 30 px (${a1 - a0}, ${b1 - b0})`);
    await s.click('[data-layer="shape-2"]');
    await s.click('[aria-label="Rotate 90° clockwise"]');
    await s.waitUntil(async () => ((await s.exec("return document.querySelector('[data-object=\"shape-2\"]').getAttribute('transform')")) ?? "").startsWith("rotate(90 "), "turned 90°");
    // A turned object keeps its handles: dragging the "se" handle 40 px down lengthens it along its
    // own (now vertical) width; its opposite corner stays where it was.
    const turned = () => s.exec("const e=document.querySelector('[data-object=\"shape-2\"]'), r=e.getBoundingClientRect(); return [Number(e.getAttribute('width')), r.top, r.height]");
    const [tw0, top0, th0] = await turned();
    const [hx, hy] = await s.exec("const r=document.querySelector('[data-corner=\"se\"]').getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]");
    await s.dragAt(hx, hy, hx, hy + 40);
    const [tw1, top1, th1] = await turned();
    const k = th0 / tw0; // screen pixels per design pixel
    assert(Math.abs(tw1 - tw0 - 40 / k) < 1.5 && Math.abs(top1 - top0) < 1 && Math.abs(th1 - th0 - 40) < 1.5, `turned resize: width ${tw0}→${tw1}, top ${top0}→${top1}, height ${th0}→${th1}`);

    // Save; the project on disk holds the edits.
    await studioSave(s, project);
    const saved = JSON.parse(readFileSync(project, "utf8"));
    const t1 = saved.objects.find((o) => o.id === "text-1");
    assert(t1.text === "SUMMER FAIR" && t1.x === x0 + 11, `saved text ${t1.text} x ${t1.x}`);
    assert(saved.objects.find((o) => o.id === "picture-1").locked && !saved.objects.find((o) => o.id === "shape-1").visible, "lock and visibility saved");
    assert(saved.objects.find((o) => o.id === "shape-2").rotation === 90, `rotation saved: ${JSON.stringify(saved.objects.filter((o) => o.kind === "shape").map(({ png, ...o }) => o))}`);
    if (CLI) {
      const svgOut = join(work, "studio-export.svg");
      execFileSync(CLI, ["design-export", project, "-o", svgOut]);
      assert(/id="shape-2" transform="rotate\(90 /.test(readFileSync(svgOut, "utf8")), "SVG export carries the turn");
      const out = join(work, "studio-export.pdf");
      execFileSync(CLI, ["design-export", project, "-o", out]);
      const text = JSON.parse(execFileSync(CLI, ["--json", "text", out], { encoding: "utf8" })).result.data.pages[0].text;
      assert(text.includes("SUMMER FAIR"), `exported text: ${text}`);
    }

    // Closing with unsaved changes asks; Cancel keeps the Studio, Don't save closes it.
    await s.click('[data-layer="text-2"]');
    await s.click('[aria-label="Delete"]');
    await s.click('[data-testid="studio-close"]');
    await s.waitFor('[role="alertdialog"]');
    await s.click('[role="alertdialog"] button:last-child');
    assert(await s.exec("return !!document.querySelector('[data-testid=studio]') && !document.querySelector('[role=alertdialog]')"), "still open after Cancel");
    await s.click('[data-testid="studio-close"]');
    await s.click('[data-testid="studio-close-discard"]');
    await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('[data-testid=studio]')")), "closed");
    assert(JSON.parse(readFileSync(project, "utf8")).objects.some((o) => o.id === "text-2"), "discarded change not saved");
    // Open in Design Studio (a picture, PDF or project) uses the system file picker, which cannot be
    // automated: it is offered, enabled, in the palette; projects are opened at launch below.
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Design Studio");
    const offered = await s.exec("const o=document.querySelector('[role=option][data-command=\"studio.open\"]'); return o ? o.getAttribute('aria-disabled') : 'missing'");
    assert(offered === null, `studio.open offered and enabled (${offered})`);
    await s.keys(["Escape"]);
  });
  // A design project given at launch opens in the Studio with the saved edits.
  await withApp([project], async (s) => {
    await s.waitFor('[data-testid="studio-layers"] [data-layer]', { timeout: 20000 });
    assert((await s.exec("return document.querySelector('[data-object=\"text-1\"]').textContent")) === "SUMMER FAIR", "reopened with edits");
    await s.keys(["Escape"]);
    await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('[data-testid=studio]')")), "Escape closes when nothing is selected or changed");
  });
});

await test("design studio: circles, ellipses and rounded boxes come back as shapes; corners and ellipse are editable", async () => {
  const src = join(work, "studio-badges.pdf");
  copyFileSync(join(FIX, "badges.pdf"), src);
  const project = join(work, "studio-badges-page1.pwdesign");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.click('.toolbar [data-command="studio.rebuildPage"]');
    await s.waitFor('[data-testid="studio-layers"] [data-layer]', { timeout: 60000 });
    const forms = () =>
      s.exec("return [...document.querySelectorAll('[data-testid=studio-stage] [data-object]')].map(e => [e.dataset.object, e.tagName.toLowerCase(), Number(e.getAttribute('rx') || 0)])");
    const f = await forms();
    const ellipses = f.filter((x) => x[1] === "ellipse");
    const rounded = f.filter((x) => x[1] === "rect" && x[2] > 0);
    assert(ellipses.length === 2 && rounded.length === 1 && !f.some((x) => x[1] === "image"), `forms ${JSON.stringify(f)}`);
    // A wide page must not shrink the Studio: the body fills the window below the bar.
    const fill = await s.exec("const b=document.querySelector('.studio-body').getBoundingClientRect(), h=document.querySelector('.studio-bar').getBoundingClientRect(); return [b.top - h.bottom, innerHeight - b.bottom]");
    assert(fill[0] === 0 && fill[1] < 40, `studio body fills the window (${fill})`);
    await axeCheck(s, "design-studio-shapes");
    writeFileSync(join(ART, "design-studio-shapes.png"), await s.screenshot());

    // The rounded box: its radius field is shown and editable; it can become an ellipse and back (undo).
    const id = rounded[0][0];
    await s.click(`[data-layer="${id}"]`);
    assert((await s.exec("return document.querySelector('[data-testid=studio-radius]').disabled")) === false, "radius editable");
    await s.exec("const i=document.querySelector('[data-testid=studio-radius]'); i.focus(); i.select(); return 1");
    await s.keys(["8"]);
    await s.waitUntil(async () => (await forms()).find((x) => x[0] === id)[2] === 8, "radius 8 drawn");
    await s.click('[data-testid="studio-ellipse"]');
    await s.waitUntil(async () => (await forms()).find((x) => x[0] === id)[1] === "ellipse", "turned into an ellipse");
    assert((await s.exec("return document.querySelector('[data-testid=studio-radius]').disabled")) === true, "radius disabled for an ellipse");
    await s.exec("document.querySelector('[data-testid=studio-stage]').focus()");
    await s.keys(["Control", "z"]);
    await s.waitUntil(async () => (await forms()).find((x) => x[0] === id)[1] === "rect", "undo restores the box");
    await studioSave(s, project);
    const saved = JSON.parse(readFileSync(project, "utf8")).objects.find((o) => o.id === id);
    assert(saved.radius === 8 && !saved.ellipse, `saved ${JSON.stringify(saved)}`);
    if (CLI) {
      const svgOut = join(work, "studio-badges.svg");
      execFileSync(CLI, ["design-export", project, "-o", svgOut]);
      const svg = readFileSync(svgOut, "utf8");
      assert((svg.match(/<ellipse /g) || []).length === 2 && new RegExp(`id="${id}"[^>]*rx="8"`).test(svg), "SVG export keeps the forms");
      const pdfOut = join(work, "studio-badges.pdf.out.pdf");
      execFileSync(CLI, ["design-export", project, "-o", pdfOut]);
      const text = JSON.parse(execFileSync(CLI, ["--json", "text", pdfOut], { encoding: "utf8" })).result.data.pages[0].text;
      assert(text.includes("SALE 50%"), `exported text: ${text}`);
    }
  });
});

await test("design studio: words over a picture become text and are painted out of the picture", async () => {
  const src = join(work, "studio-caption.pdf");
  copyFileSync(join(FIX, "caption.pdf"), src);
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.click('.toolbar [data-command="studio.rebuildPage"]');
    await s.waitFor('[data-testid="studio-layers"] [data-layer]', { timeout: 60000 });
    assert((await s.text('[data-testid="studio-summary"]')).includes("2 text · 0 shapes · 1 pictures"), "summary counts");
    const words = await s.exec("return [...document.querySelectorAll('[data-testid=studio-stage] text')].map(t => t.textContent)");
    assert(words.includes("OPEN DAY") && words.includes("Saturday from ten"), `words ${words}`);
    await s.click('[data-layer="text-1"]');
    assert((await s.text(".studio-note")).includes("painted out"), "the note says the wording was painted out");
    // Font family and slant: drawn on the canvas, saved, and exported (PNG at the design's size).
    await s.exec("const e=document.querySelector('[data-testid=studio-font]'); e.focus(); return 1");
    await s.click('[data-testid="studio-font"]');
    await s.click('[data-testid="studio-font"] option[value="serif"]');
    await s.click('[data-testid="studio-italic"]');
    await s.waitUntil(async () => (await s.exec("const t=document.querySelector('[data-object=\"text-1\"]'); return t.getAttribute('font-style') + '|' + t.getAttribute('font-family')")).startsWith("italic|'Times"), "serif italic drawn");
    const projectFile = join(work, "studio-caption-page1.pwdesign");
    await studioSave(s, projectFile);
    const t1 = JSON.parse(readFileSync(projectFile, "utf8")).objects.find((o) => o.id === "text-1");
    assert(t1.font === "serif" && t1.italic === true, `saved font ${t1.font} italic ${t1.italic}`);
    if (CLI) {
      const out = join(work, "studio-caption.png");
      execFileSync(CLI, ["design-export", projectFile, "-o", out]);
      const png = readFileSync(out);
      const [w, h] = [png.readUInt32BE(16), png.readUInt32BE(20)];
      const d = JSON.parse(readFileSync(projectFile, "utf8"));
      assert(png.subarray(1, 4).toString() === "PNG" && w === d.width && h === d.height, `PNG ${w}×${h}`);
    }
    // Hide both lines: the picture underneath shows no words.
    for (const name of ["OPEN DAY", "Saturday from ten"]) {
      await s.click(`[data-layer="${await s.exec(`return [...document.querySelectorAll('[data-layer]')].find(b => b.textContent.includes(${JSON.stringify(name)})).dataset.layer`)}"]`);
      await s.click(`[aria-label="Hide ${name}"]`);
    }
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('[data-testid=studio-stage] text').length")) === 0, "text hidden");
    writeFileSync(join(ART, "design-studio-painted-out.png"), await s.screenshot());
    // Crop the picture: 20 px off the left and 10 off the bottom; the rest stays where it was.
    await s.click('[data-layer="picture-1"]');
    const pic = () => s.exec("const e=document.querySelector('[data-object=\"picture-1\"]'); return ['x','y','width','height'].map(a => Number(e.getAttribute(a)))");
    const [px0, py0, pw0, ph0] = await pic();
    for (const [side, v] of [["l", "20"], ["b", "10"]]) {
      await s.exec(`const i=document.querySelector('[data-crop="${side}"]'); i.focus(); i.select(); return 1`);
      await s.keys([...v]);
    }
    await s.waitFor('[data-testid="studio-crop-box"]');
    await s.click('[data-testid="studio-crop-apply"]');
    await s.waitUntil(async () => (await pic())[2] === pw0 - 20, "cropped");
    const [px1, py1, , ph1] = await pic();
    assert(px1 === px0 + 20 && py1 === py0 && ph1 === ph0 - 10, `crop kept the rest in place (${px0},${py0},${pw0},${ph0} → ${await pic()})`);
    assert(!(await s.exec("return !!document.querySelector('[data-testid=studio-crop-box]')")), "preview gone after cropping");
  });
});

await test("design studio: borders and rings come back as outlined shapes; outline and fill are editable", async () => {
  const src = join(work, "studio-frames.pdf");
  copyFileSync(join(FIX, "frames.pdf"), src);
  const project = join(work, "studio-frames-page1.pwdesign");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.click('.toolbar [data-command="studio.rebuildPage"]');
    await s.waitFor('[data-testid="studio-layers"] [data-layer]', { timeout: 60000 });
    const shapes = () => s.exec("return [...document.querySelectorAll('[data-testid=studio-stage] [data-object^=shape]')].map(e => [e.dataset.object, e.tagName.toLowerCase(), e.getAttribute('fill'), Number(e.getAttribute('stroke-width'))])");
    const f = await shapes();
    assert(f.length === 2 && f.every((x) => x[2] === "none" && x[3] > 3) && f.some((x) => x[1] === "ellipse"), `frames ${JSON.stringify(f)}`);
    await axeCheck(s, "design-studio-frames");
    writeFileSync(join(ART, "design-studio-frames.png"), await s.screenshot());
    // Zoom in from the keyboard: the page grows and its top-left corner stays reachable by scrolling.
    await s.exec("document.querySelector('[data-testid=studio-stage]').focus()");
    const stageW = () => s.exec("return document.querySelector('[data-testid=studio-stage]').getBoundingClientRect().width");
    const w0 = await stageW();
    for (let i = 0; i < 4; i++) await s.keys(["Control", "="]);
    await s.waitUntil(async () => (await stageW()) > w0 * 2, "zoomed in");
    const reach = await s.exec("const c=document.querySelector('[data-testid=studio-canvas]'); c.scrollLeft=0; c.scrollTop=0; const a=c.getBoundingClientRect(), b=document.querySelector('[data-testid=studio-stage]').getBoundingClientRect(); return [b.left - a.left, b.top - a.top, c.scrollWidth > c.clientWidth]");
    assert(reach[0] >= 0 && reach[1] >= 0 && reach[2], `zoomed page reachable from its top-left (${reach})`);
    // Pan: hold Space and drag left by 120 px → the canvas scrolls right by 120.
    const [cx, cy] = await s.exec("const r=document.querySelector('[data-testid=studio-canvas]').getBoundingClientRect(); return [Math.round(r.left + r.width / 2), Math.round(r.top + r.height / 2)]");
    await s.cmd("POST", "/actions", { actions: [
      { type: "key", id: "kb", actions: [{ type: "keyDown", value: " " }, { type: "pause", duration: 0 }, { type: "pause", duration: 0 }, { type: "pause", duration: 0 }, { type: "pause", duration: 0 }, { type: "keyUp", value: " " }] },
      { type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [
        { type: "pointerMove", origin: "viewport", x: cx, y: cy },
        { type: "pointerDown", button: 0 },
        { type: "pointerMove", origin: "viewport", duration: 150, x: cx - 120, y: cy },
        { type: "pointerUp", button: 0 },
        { type: "pause", duration: 0 },
      ] },
    ] });
    await s.cmd("DELETE", "/actions");
    const scrolled = await s.exec("return document.querySelector('[data-testid=studio-canvas]').scrollLeft");
    assert(Math.abs(scrolled - 120) <= 2, `Space+drag panned (${scrolled})`);
    await s.keys(["Control", "0"]);
    await s.waitUntil(async () => Math.abs((await stageW()) - w0) < 1, "back to fit");
    const border = f.find((x) => x[1] === "rect")[0];
    await s.click(`[data-layer="${border}"]`);
    // Fill the frame, then take the fill away again; thicken the outline.
    await s.click('[data-testid="studio-no-fill"]');
    await s.waitUntil(async () => (await shapes()).find((x) => x[0] === border)[2] !== "none", "filled");
    await s.click('[data-testid="studio-no-fill"]');
    await s.waitUntil(async () => (await shapes()).find((x) => x[0] === border)[2] === "none", "fill removed");
    await s.exec("const i=document.querySelector('[data-testid=studio-stroke-width]'); i.focus(); i.select(); return 1");
    await s.keys(["1", "2"]);
    await s.waitUntil(async () => (await shapes()).find((x) => x[0] === border)[3] === 12, "outline 12 px");
    await studioSave(s, project);
    const saved = JSON.parse(readFileSync(project, "utf8")).objects.find((o) => o.id === border);
    assert(saved.fill === undefined && saved.stroke_width === 12, `saved ${JSON.stringify(saved)}`);
    if (CLI) {
      const out = join(work, "studio-frames.svg");
      execFileSync(CLI, ["design-export", project, "-o", out]);
      assert(new RegExp(`id="${border}"[^>]*fill="none" stroke="[^"]+" stroke-width="12"`).test(readFileSync(out, "utf8")), "SVG keeps the outline");
    }
  });
});

await test("images to PDF: opens from the start screen; explains what is missing; accessible", async () => {
  await withApp([], async (s) => {
    await s.click('[data-command="file.imagesToPdf"]');
    await s.waitFor('[data-testid="images-start"]');
    assert((await s.attr('[data-testid="images-start"]', "title")) === "Add images first.", "disabled with a reason");
    assert((await s.text('[role="dialog"] h2')) === "Create PDF from images", "dialog title");
    await axeCheck(s, "images-to-pdf-dialog");
    writeFileSync(join(ART, "images-to-pdf-dialog.png"), await s.screenshot());
    await s.keys(["Escape"]);
    await s.waitUntil(async () => (await s.exec("return !!document.querySelector('[role=dialog]')")) === false, "closed");
  });
  if (CLI) {
    // The conversion itself, through the same engine (the native file picker cannot be automated).
    const out = join(work, "from-images.pdf");
    execFileSync(CLI, ["images-to-pdf", join(FIX, "../images/photo-exif-rotate.jpg"), join(FIX, "../images/alpha.png"), "-o", out]);
    const v = JSON.parse(execFileSync(CLI, ["--json", "inspect", out], { encoding: "utf8" })).result.data;
    assert(v.page_count === 2 && v.pages[0].rotation === 90, `converted: ${JSON.stringify(v.pages.map((p) => p.rotation))}`);
  }
});

await test("images given at launch go to Create PDF from images (both listed); the PDF is made", async () => {
  const dir = mkdtempSync(join(work, "launch-images-"));
  const a = join(dir, "first.jpg");
  const b = join(dir, "second.png");
  copyFileSync(join(FIX, "../images/photo-exif-rotate.jpg"), a);
  copyFileSync(join(FIX, "../images/alpha.png"), b);
  const out = join(dir, "together.pdf");
  await withApp([a, b], async (s) => {
    await s.waitFor('[data-testid="images-start"]', { timeout: 20000 });
    await s.waitUntil(async () => (await s.exec("return [...document.querySelectorAll('.file-list .name')].map(n => n.textContent).join('|')")) === "1. first.jpg|2. second.png", "both pictures listed in order");
    assert(!(await s.exec("return !!document.querySelector('.notice')?.innerText.includes('not a PDF')")), "no 'not a PDF' error");
    await s.type("#images-output", out);
    await s.click('[data-testid="images-start"]');
    await s.waitUntil(() => existsSync(out), "PDF written", 60000);
  });
  if (CLI) {
    const v = JSON.parse(execFileSync(CLI, ["--json", "inspect", out], { encoding: "utf8" })).result.data;
    assert(v.page_count === 2, `two pages (${v.page_count})`);
  }
});

await test("bookmarks: add from the page menu, rename, nest, reorder, delete; saved outline read back exactly", async () => {
  const src = join(FIX, "five-pages.pdf");
  const before = sha(src);
  const out = join(work, "outline.pdf");
  const titles = (s) => s.exec("return [...document.querySelectorAll('[data-bookmark-title]')].map(i => i.value)");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.contextClick('[data-page="0"]');
    await s.waitFor('[role="menu"]');
    assert((await s.text('[role="menuitem"][data-command="file.editBookmarks"]')).startsWith("Add bookmark for page 1"), "menu entry names the page");
    await s.click('[role="menuitem"][data-command="file.editBookmarks"]');
    await s.waitFor('[data-bookmark-title="5"]');
    assert(JSON.stringify(await titles(s)) === JSON.stringify(["Chapter 1", "Chapter 2", "Chapter 3", "Chapter 4", "Chapter 5", "Page 1"]), `rows: ${await titles(s)}`);
    // Rename the new one and make it a sub-bookmark of Chapter 5.
    await s.type('[data-bookmark-title="5"]', "Summary");
    await s.click('[data-testid="bookmark-indent"]');
    // A sub-bookmark cannot move above its parent's siblings: Move up is disabled with a reason.
    assert((await s.attr('[data-testid="bookmark-up"]', "aria-disabled")) === "true", "move up disabled for an only child");
    // Delete Chapter 2, then move Chapter 1 down below Chapter 3.
    await s.click('[data-bookmark-title="1"]');
    await s.click('[data-testid="bookmark-delete"]');
    await s.click('[data-bookmark-title="0"]');
    await s.click('[data-testid="bookmark-down"]');
    assert(JSON.stringify(await titles(s)) === JSON.stringify(["Chapter 3", "Chapter 1", "Chapter 4", "Chapter 5", "Summary"]), `after edits: ${await titles(s)}`);
    // An out-of-range page blocks saving with a reason.
    await s.replaceText('[data-bookmark-page="0"]', "9");
    await s.waitUntil(async () => ((await s.attr('[data-testid="bookmarks-save"]', "title")) ?? "").includes("from 1 to 5"), "page range explained");
    await s.replaceText('[data-bookmark-page="0"]', "3");
    await s.type("#bookmarks-output", out);
    await axeCheck(s, "edit-bookmarks-dialog");
    writeFileSync(join(ART, "edit-bookmarks-dialog.png"), await s.screenshot());
    await s.waitUntil(async () => (await s.attr('[data-testid="bookmarks-save"]', "aria-disabled")) === null, "ready");
    await s.click('[data-testid="bookmarks-save"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved outline.pdf")), "saved notice", 30000);
  });
  assert(sha(src) === before, "source PDF unchanged");
  if (CLI) {
    const b = JSON.parse(execFileSync(CLI, ["--json", "inspect", out], { encoding: "utf8" })).result.data.bookmarks;
    const got = b.map((x) => `${x.depth}:${x.title}@${x.page}`).join("|");
    assert(got === "0:Chapter 3@2|0:Chapter 1@0|0:Chapter 4@3|0:Chapter 5@4|1:Summary@0", `outline read back: ${got}`);
  }
});

await test("attachments: the sidebar lists embedded files; one is saved exactly as stored", async () => {
  const src = join(FIX, "hidden-info.pdf");
  const before = sha(src);
  const out = join(work, "from-pdf.txt");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.click("#sb-tab-attachments");
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[data-attachment=\"0\"] .name')?.textContent ?? null")) === "secret.txt", "embedded file listed");
    await axeCheck(s, "attachments-panel");
    await s.click('button[aria-label="Save secret.txt"]');
    await s.waitFor("#attachment-output");
    assert((await s.exec("return document.querySelector('#attachment-output').value")).endsWith("secret.txt"), "default name from the attachment");
    await s.type("#attachment-output", out);
    await axeCheck(s, "save-attachment-dialog");
    await s.click('[data-testid="attachment-save"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved from-pdf.txt")), "saved notice", 30000);
    const texts = await noticeTexts(s);
    assert(texts.some((t) => t.includes("trust")), "warns to open only trusted files");
    // A text file is not opened in PageWeave: the notice offers Show in folder, not Open.
    const actions = await s.exec("return [...document.querySelectorAll('.notice')].filter(n => n.textContent.includes('from-pdf.txt')).flatMap(n => [...n.querySelectorAll('button')].map(b => b.textContent))");
    assert(!actions.includes("Open") && actions.includes("Show in folder"), `notice actions: ${actions}`);
  });
  assert(sha(src) === before, "source PDF unchanged");
  assert(readFileSync(out, "utf8") === "Embedded PWATTACH-DOC", "bytes exactly as stored");
});

await test("crop pages: margins in millimetres, shaded preview, rotated page cropped as displayed, verified", async () => {
  const src = join(FIX, "mixed-sizes-rotated.pdf");
  const before = sha(src);
  const out = join(work, "trimmed.pdf");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Crop pages");
    await s.click('[role="option"][data-command="pages.crop"]');
    await s.waitFor("#crop-top");
    assert((await s.attr('[data-testid="tool-start"]', "title")) === "Enter at least one margin to crop.", "disabled with a reason");
    await s.type("#crop-top", "25.4");
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[data-testid=crop-preview] .cut').style.height")) !== "0px", "preview shades the top");
    await s.type("#crop-left", "500");
    await s.waitUntil(async () => (await s.attr('[data-testid="tool-start"]', "title")) === "The margins leave almost nothing of the page.", "too much refused");
    await s.type("#crop-left", "0");
    await s.type("#output", out);
    await axeCheck(s, "crop-dialog");
    writeFileSync(join(ART, "crop-dialog.png"), await s.screenshot());
    await s.waitUntil(async () => (await s.attr('[data-testid="tool-start"]', "aria-disabled")) === null, "ready");
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved trimmed.pdf")), "saved notice", 30000);
    assert((await noticeTexts(s)).some((t) => t.includes("hidden, not removed")), "says cropping does not remove content");
  });
  assert(sha(src) === before, "source PDF unchanged");
  if (CLI) {
    const pages = JSON.parse(execFileSync(CLI, ["--json", "inspect", out], { encoding: "utf8" })).result.data.pages;
    // 25.4 mm = 72 pt off the displayed top: page 1 (letter) gets shorter; page 3 (/Rotate 90) loses it from its left side.
    assert(Math.abs(pages[0].height_pt - 720) < 0.1 && Math.abs(pages[0].width_pt - 612) < 0.1, `page 1: ${JSON.stringify(pages[0])}`);
    assert(Math.abs(pages[2].width_pt - 540) < 0.1 && Math.abs(pages[2].height_pt - 792) < 0.1, `page 3: ${JSON.stringify(pages[2])}`);
  }
});

await test("watermark: Pages menu, preview follows angle, chosen pages only, translucent result verified", async () => {
  const src = join(FIX, "five-pages.pdf");
  const before = sha(src);
  const out = join(work, "stamped-draft.pdf");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.exec("[...document.querySelectorAll('button[aria-haspopup=menu]')].find(b => b.textContent.trim() === 'Pages').click(); return 1");
    await s.click('[role="menuitem"][data-command="pages.watermark"]');
    await s.waitFor("#wm-text");
    assert((await s.text('[role="dialog"] h2')) === "Watermark", "dialog title");
    assert((await s.exec("return document.querySelector('#output').value")).endsWith("five-pages-watermarked.pdf"), "default output name");
    await s.type("#wm-text", "DRAFT COPY");
    await s.click('[role="radio"][data-angle="0"]');
    const transform = () => s.exec("return document.querySelector('[data-testid=wm-preview] text').getAttribute('transform')");
    assert(/^rotate\(-?0 48 62\)$/.test(await transform()), `preview follows the angle: ${await transform()}`);
    await s.click('[role="radio"][data-angle="45"]');
    assert((await transform()) === "rotate(-45 48 62)", "preview at 45°");
    assert((await s.attr('[data-testid="wm-preview"]', "aria-label")).includes("45°"), "preview has an accessible description");
    // Only pages 2 and 4.
    await s.click('[role="dialog"] input[type=checkbox]');
    await s.type("#pages", "2, 4");
    await s.waitUntil(async () => (await s.text("#pages-msg")) === "2 pages selected", "range checked");
    await s.type("#output", out);
    await axeCheck(s, "watermark-dialog");
    writeFileSync(join(ART, "watermark-dialog.png"), await s.screenshot());
    await s.waitUntil(async () => (await s.attr('[data-testid="tool-start"]', "aria-disabled")) === null, "ready");
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved stamped-draft.pdf")), "saved notice", 30000);
    // Open the copy: the watermark is findable on page 2.
    await s.exec("[...document.querySelectorAll('.notice button')].find(b => b.textContent === 'Open').click(); return 1");
    await s.waitUntil(async () => (await s.exec("return [...document.querySelectorAll('.tab-label .name')].map(n => n.textContent)")).includes("stamped-draft.pdf"), "copy opened");
    await pageLoaded(s, 0);
    await s.keys(["Control", "f"]);
    await s.type('input[aria-label="Find in document"]', "DRAFT");
    await s.keys(["Enter"]);
    await s.waitUntil(async () => /^1 of 2/.test(await s.text(".searchbar .count")), "found on two pages");
    await s.keys(["Escape"]);
  });
  assert(sha(src) === before, "source PDF unchanged");
  if (CLI) {
    const r = JSON.parse(execFileSync(CLI, ["--json", "search", out, "DRAFT"], { encoding: "utf8" })).result.data;
    assert(r.count === 2 && r.hits.map((h) => h.page).join() === "1,3", `independent check: ${JSON.stringify(r.hits.map((h) => h.page))}`);
  }
});

await test("document properties: edit from the properties dialog; saved copy shows the new values; unchanged values kept", async () => {
  const src = join(FIX, "five-pages.pdf");
  const before = sha(src);
  const out = join(work, "retitled.pdf");
  const title = "Retitled — Überblick";
  const propRows = (s) => s.exec("return Object.fromEntries([...document.querySelectorAll('[role=dialog] tr')].map(r => [r.querySelector('th').textContent, r.querySelector('td').textContent]))");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.keys(["Control", "d"]);
    await s.waitFor('[data-command="file.editProperties"]');
    await s.click('[data-command="file.editProperties"]');
    await s.waitFor("#prop-title");
    assert((await s.exec("return document.querySelector('#prop-title').value")) === "Five pages", "prefilled from the file");
    assert((await s.attr('[data-testid="props-start"]', "title")) === "Change a property first.", "disabled until something changes");
    await s.type("#prop-title", title);
    // Empty the author field with real keys (select all, delete).
    await s.click("#prop-author");
    await s.keys(["Control", "a"]);
    await s.keys(["Backspace"]);
    await s.waitUntil(async () => (await s.exec("return document.querySelector('#prop-author').value")) === "", "author emptied");
    await s.type("#prop-subject", "Quarterly");
    await s.type("#props-output", out);
    await axeCheck(s, "edit-properties-dialog");
    writeFileSync(join(ART, "edit-properties-dialog.png"), await s.screenshot());
    await s.waitUntil(async () => (await s.attr('[data-testid="props-start"]', "aria-disabled")) === null, "ready");
    await s.click('[data-testid="props-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved retitled.pdf")), "saved notice", 30000);
    await s.exec("[...document.querySelectorAll('.notice button')].find(b => b.textContent === 'Open').click(); return 1");
    await s.waitUntil(async () => (await s.exec("return [...document.querySelectorAll('.tab-label .name')].map(n => n.textContent)")).includes("retitled.pdf"), "copy opened");
    await pageLoaded(s, 0);
    await s.keys(["Control", "d"]);
    await s.waitFor('[role="dialog"] tr');
    const rows = await propRows(s);
    assert(rows.Title === title && rows.Author === "—" && rows.Subject === "Quarterly" && rows.Bookmarks === "5", `properties of the copy: ${JSON.stringify(rows)}`);
    await s.keys(["Escape"]);
  });
  assert(sha(src) === before, "source PDF unchanged");
  if (CLI) {
    const v = JSON.parse(execFileSync(CLI, ["--json", "inspect", out], { encoding: "utf8" })).result.data;
    assert(v.title === title && v.author === null && v.subject === "Quarterly" && v.page_count === 5, `independent check: ${JSON.stringify(v).slice(0, 200)}`);
  }
});

await test("remove password: copy opens without a password; unavailable with a reason on unprotected files; owner restrictions honoured", async () => {
  const src = join(FIX, "encrypted-rc4-user-password.pdf");
  const before = sha(src);
  const out = join(work, "unlocked.pdf");
  const restricted = join(work, "restricted.pdf");
  if (CLI) {
    const r = spawnSync(CLI, ["protect", "--no-modify", "--owner-password-stdin", "-o", restricted, join(FIX, "five-pages.pdf")], { input: "reader\nowner\n" });
    assert(r.status === 0, "restricted test file made by the CLI");
  }
  const openPalette = async (s, text) => {
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', text);
    await s.waitFor('[role="option"][data-command="file.unprotect"]');
  };
  // withApp waits for a tab, so an unprotected file opens alongside the protected one.
  await withApp([join(FIX, "single-page.pdf"), src], async (s) => {
    await s.waitFor("#pdf-password");
    await s.type("#pdf-password", "pw-user-test");
    await s.keys(["Enter"]);
    // Wait for the protected file's tab (the first tab may already show a page; found on Windows CI).
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('.tab-label .name').length")) === 2, "both open");
    await pageLoaded(s, 0);
    await openPalette(s, "Remove password");
    await s.click('[role="option"][data-command="file.unprotect"]');
    await s.waitFor("#unprotect-output");
    assert((await s.exec("return document.querySelector('#unprotect-output').value")).endsWith("encrypted-rc4-user-password-unprotected.pdf"), "default output name");
    await s.type("#unprotect-output", out);
    await axeCheck(s, "unprotect-dialog");
    await s.click('[data-testid="unprotect-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved unlocked.pdf")), "saved notice", 30000);
    // The copy opens straight away: no password prompt.
    await s.exec("[...document.querySelectorAll('.notice button')].find(b => b.textContent === 'Open').click(); return 1");
    await s.waitUntil(async () => (await s.exec("return [...document.querySelectorAll('.tab-label .name')].map(n => n.textContent)")).includes("unlocked.pdf"), "copy opened");
    await pageLoaded(s, 0);
    assert((await s.exec("return !!document.querySelector('#pdf-password')")) === false, "no password asked");
    // On the unprotected copy the command is unavailable and says why.
    await openPalette(s, "Remove password");
    const reason = await s.exec("const o = document.querySelector('[data-command=\"file.unprotect\"]'); return o.getAttribute('aria-disabled') + '|' + o.textContent");
    assert(reason.startsWith("true|") && reason.includes("not password protected"), `disabled with a reason: ${reason}`);
    await s.keys(["Escape"]);
  });
  assert(sha(src) === before, "source PDF unchanged");
  if (CLI) {
    // A file with owner restrictions, opened with its open password: refused, nothing written.
    const v = JSON.parse(execFileSync(CLI, ["--json", "inspect", out], { encoding: "utf8" })).result.data;
    assert(v.encrypted === false && v.page_count === 5, `independent check: ${JSON.stringify(v).slice(0, 200)}`);
    await withApp([join(FIX, "single-page.pdf"), restricted], async (s) => {
      await s.waitFor("#pdf-password");
      await s.type("#pdf-password", "reader");
      await s.keys(["Enter"]);
      await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('.tab-label .name').length")) === 2, "both open");
      await pageLoaded(s, 0);
      await openPalette(s, "Remove password");
      await s.click('[role="option"][data-command="file.unprotect"]');
      await s.waitFor("#unprotect-output");
      await s.click('[data-testid="unprotect-start"]');
      await s.waitUntil(async () => ((await s.exec("return document.querySelector('[role=dialog] .banner.error')?.textContent ?? ''")) || "").includes("restricted"), "restriction explained", 30000);
      assert(!existsSync(join(work, "restricted-unprotected.pdf")), "nothing written");
    });
  }
});

await test("remove hidden information: findings listed, verified copy has none, kept content stays; protected files refused with a reason", async () => {
  const src = join(FIX, "hidden-info.pdf");
  const before = sha(src);
  const out = join(work, "cleaned.pdf");
  // Null until the dialog has rendered (the palette closes first), so waitUntil keeps polling.
  const finding = (s, k) => s.exec(`return document.querySelector('[data-finding=${k}]')?.textContent ?? null`);
  await withApp([src, join(FIX, "encrypted-rc4-user-password.pdf")], async (s) => {
    // The protected file opens with its password first; the dialog refuses it with a reason.
    await s.waitFor("#pdf-password");
    await s.type("#pdf-password", "pw-user-test");
    await s.keys(["Enter"]);
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('.tab-label .name').length")) === 2, "both open");
    await pageLoaded(s, 0);
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Remove hidden");
    await s.click('[role="option"][data-command="file.sanitize"]');
    await s.waitUntil(async () => ((await s.attr('[data-testid="sanitize-start"]', "title")) ?? "").includes("Password-protected"), "protected file refused with a reason");
    await s.keys(["Escape"]);
    // Switch to the file with hidden information.
    await s.exec("[...document.querySelectorAll('.tab-label .name')].find(n => n.textContent === 'hidden-info.pdf').click(); return 1");
    await pageLoaded(s, 0);
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Remove hidden");
    await s.click('[role="option"][data-command="file.sanitize"]');
    await s.waitUntil(async () => (await finding(s, "scripts")) === "7 found", "findings listed");
    assert((await finding(s, "metadata")).startsWith("Title, Author"), "metadata kinds listed");
    assert((await finding(s, "attachments")) === "2 found: notes.txt, secret.txt", "attachment names");
    assert((await finding(s, "hidden_layers")) === "Draft notes", "hidden layer named");
    assert((await finding(s, "form_fields")) === "1 found", "field found");
    assert((await s.exec("return document.querySelector('input[data-kind=form_fields]').checked")) === false, "fields kept by default");
    assert((await s.text('[data-testid="sanitize-always"]')).includes("were found"), "earlier versions reported");
    assert((await s.exec("return document.querySelector('#sanitize-output').value")).endsWith("hidden-info-cleaned.pdf"), "default output name");
    await s.click("input[data-kind=web_links]");
    await s.type("#sanitize-output", out);
    await axeCheck(s, "sanitize-dialog");
    writeFileSync(join(ART, "sanitize-dialog.png"), await s.screenshot());
    await s.click('[data-testid="sanitize-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved cleaned.pdf")), "saved notice", 30000);
    // Open the copy and inspect it again: nothing selected is left.
    await s.exec("[...document.querySelectorAll('.notice button')].find(b => b.textContent === 'Open').click(); return 1");
    await s.waitUntil(async () => (await s.exec("return [...document.querySelectorAll('.tab-label .name')].map(n => n.textContent)")).includes("cleaned.pdf"), "copy opened");
    await pageLoaded(s, 0);
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Remove hidden");
    await s.click('[role="option"][data-command="file.sanitize"]');
    await s.waitUntil(async () => (await finding(s, "scripts")) === "None found", "copy re-inspected");
    for (const k of ["metadata", "attachments", "hidden_layers", "comments", "web_links"]) {
      assert((await finding(s, k)) === "None found", `${k} gone`);
    }
    assert((await finding(s, "form_fields")) === "1 found", "field kept");
    await s.keys(["Escape"]);
  });
  assert(sha(src) === before, "source PDF unchanged");
  if (CLI) {
    const t = JSON.parse(execFileSync(CLI, ["--json", "text", "--pages", "1", out], { encoding: "utf8" })).result.data.pages[0].text;
    assert(t.includes("PWBODY") && t.includes("PWLAYER-SHOWN") && !t.includes("PWLAYER-HIDDEN"), `independent text check: ${t.slice(0, 200)}`);
    const f = JSON.parse(execFileSync(CLI, ["--json", "sanitize", "--inspect", out], { encoding: "utf8" })).result.data;
    assert(f.scripts === 0 && f.attachments === 0 && f.comments === 0 && f.web_links === 0 && !f.earlier_versions && f.metadata.length === 0, `independent inspection: ${JSON.stringify(f)}`);
  }
});

await test("OCR: a scanned file becomes searchable from the Pages menu; the copy opens with selectable text", async () => {
  const src = join(FIX, "scanned.pdf");
  const before = sha(src);
  const out = join(work, "searchable.pdf");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    // Nothing to find before recognition.
    await s.keys(["Control", "f"]);
    await s.type('input[aria-label="Find in document"]', "Description");
    await s.keys(["Enter"]);
    await s.waitUntil(async () => (await s.text(".searchbar .count")) === "No matches", "no text before OCR");
    await s.keys(["Escape"]);
    // Pages menu → Recognize text (OCR)…
    await s.exec("[...document.querySelectorAll('button[aria-haspopup=menu]')].find(b => b.textContent.trim() === 'Pages').click(); return 1");
    await s.click('[role="menuitem"][data-command="pages.ocr"]');
    await s.waitFor("#output");
    assert((await s.text('[role="dialog"] h2')) === "Recognize text (OCR)", "dialog title");
    assert((await s.exec("return document.querySelector('#output').value")).endsWith("scanned-searchable.pdf"), "default output name");
    assert((await s.text('[data-testid="tool-start"]')) === "Recognize", "start button label");
    await axeCheck(s, "ocr-dialog");
    await s.type("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved searchable.pdf")), "saved notice", 240000);
    // Open the copy from the notice and find text in it.
    await s.exec("[...document.querySelectorAll('.notice button')].find(b => b.textContent === 'Open').click(); return 1");
    await s.waitUntil(async () => (await s.exec("return [...document.querySelectorAll('.tab-label .name')].map(n => n.textContent)")).includes("searchable.pdf"), "copy opened");
    await pageLoaded(s, 0);
    await s.keys(["Control", "f"]);
    await s.type('input[aria-label="Find in document"]', "Description");
    await s.keys(["Enter"]);
    await s.waitUntil(async () => /^1 of 1/.test(await s.text(".searchbar .count")), "found after OCR");
    await s.keys(["Escape"]);
    // The recognised text is selectable in the viewer's text layer.
    await markSpan(s, 0, "Invoice");
    await s.doubleClickAt(...(await s.exec("const r=document.querySelector('[data-e2e=target]').getBoundingClientRect(); return [r.left + 8, r.top + r.height/2]")));
    await s.waitUntil(async () => (await selection(s)).trim().length > 0, "word selectable");
  });
  assert(existsSync(out), "searchable copy exists");
  assert(sha(src) === before, "source PDF unchanged");
  if (CLI) {
    const t = JSON.parse(execFileSync(CLI, ["--json", "text", "--pages", "1", out], { encoding: "utf8" })).result.data.pages[0].text;
    assert(t.includes("Invoice") && t.includes("Description"), `independent text check: ${t.slice(0, 200)}`);
  }
});

await test("redaction: mark by search, selection and dragged area; review; apply a verified copy with the content gone", async () => {
  const src = join(FIX, "redaction.pdf");
  const before = sha(src);
  const out = join(work, "redacted.pdf");
  await withApp([src], async (s) => {
    const marks = () => s.exec("return document.querySelector('[data-testid=redaction-marks]')?.textContent ?? ''");
    await pageLoaded(s, 0);
    // Apply is unavailable, with a reason, until something is marked.
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Apply redactions");
    const applyOpt = await s.exec("const o=document.querySelector(`[role=option][data-command='${arguments[0]}']`); return o ? o.getAttribute('aria-disabled') + '|' + o.textContent : null", "redact.apply");
    assert(applyOpt && applyOpt.startsWith("true") && applyOpt.includes("Mark text or areas"), `apply unavailable without marks (${applyOpt})`);
    await s.keys(["Escape"]);
    // 1. Find → Mark all for redaction (disabled with a reason before there are matches).
    await s.keys(["Control", "f"]);
    const reason = await s.attr('[data-command="redact.markMatches"]', "title");
    assert(reason.includes("Find text first"), `mark-all explains why it is unavailable (${reason})`);
    await s.type('input[aria-label="Find in document"]', "SECRET-4711");
    await s.keys(["Enter"]);
    await s.waitUntil(async () => (await s.attr('[data-command="redact.markMatches"]', "aria-disabled")) === null, "mark-all enabled once there are matches");
    await s.click('[data-command="redact.markMatches"]');
    await s.waitUntil(async () => (await annotCount(s, "redact")) === 1, "search match marked");
    await s.keys(["Escape"]);

    // 2. Selected word → Ctrl+Shift+X.
    await markSpan(s, 0, "PWHIDE");
    await s.doubleClickAt(...(await s.exec("const r=document.querySelector('[data-e2e=target]').getBoundingClientRect(); return [r.right - 20, r.top + r.height/2]")));
    await s.waitUntil(async () => (await selection(s)).trim() === "PWHIDE", "word selected");
    await s.keys(["Control", "Shift", "X"]);
    await s.waitUntil(async () => (await annotCount(s, "redact")) === 2, "selection marked");

    // 3. Mark areas mode (command palette) → drag over the image at 450–482 × 300–332 pt.
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Mark areas");
    await s.click('[role="option"][data-command="redact.markArea"]');
    await s.waitFor('[data-testid="mark-mode"]');
    // Page point (x, y) in PDF space → viewport pixels, measured after scrolling settles.
    await s.exec("document.querySelector('.viewer [data-page=\"0\"]').scrollIntoView({block:'center'}); return 1");
    await new Promise((r) => setTimeout(r, 200));
    const box = await s.exec("const r=document.querySelector('.viewer [data-page=\"0\"]').getBoundingClientRect(); return [r.left, r.top, r.width/612]");
    const at = (x, y) => [box[0] + x * box[2], box[1] + (792 - y) * box[2]];
    await s.dragAt(...at(444, 344), ...at(488, 294));
    await s.waitUntil(async () => (await annotCount(s, "redact")) === 3, "dragged area marked");
    assert((await selection(s)) === "", "dragging in mark mode does not select text");
    await s.keys(["Escape"]);
    await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('[data-testid=mark-mode]')")), "Escape ends mark mode");
    await s.waitUntil(async () => (await marks()) === "3 redaction marks — Apply…", "status bar counts the marks");
    await axeCheck(s, "viewer-with-redaction-marks");
    writeFileSync(join(ART, "redaction-marks.png"), await s.screenshot());

    // 4. Right-click a mark → Remove; the notice's Undo brings it back.
    await s.contextClickAt(...at(466, 316));
    await s.waitFor('[role="menuitem"]');
    const labels = await s.exec("return [...document.querySelectorAll('[role=menuitem]')].map(m => m.textContent)");
    assert(labels.some((l) => l.startsWith("Remove this redaction mark")), `menu offers removing the mark (${labels})`);
    await s.exec("[...document.querySelectorAll('[role=menuitem]')].find(m => m.textContent.startsWith('Remove this redaction mark')).click(); return 1");
    await s.waitUntil(async () => (await annotCount(s, "redact")) === 2, "mark removed");
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Redaction mark removed")), "removal notice");
    await s.exec("[...document.querySelectorAll('.notice button')].find(b => b.textContent === 'Undo').click(); return 1");
    await s.waitUntil(async () => (await annotCount(s, "redact")) === 3, "Undo restores the mark");

    // Whole page: page 2's context menu → Mark page 2 for redaction.
    await s.exec("document.querySelector('.viewer [data-page=\"1\"]').scrollIntoView({block:'center'}); return 1");
    await s.contextClick('.viewer [data-page="1"]', 0, 0);
    await s.click('[role="menuitem"][data-command="redact.markPage"]');
    await s.waitUntil(async () => (await marks()) === "4 redaction marks — Apply…", "whole page marked");

    // 5. Closing the tab asks; marks cannot be saved into a copy, and the prompt says so.
    await s.keys(["Control", "w"]);
    await s.waitFor('[data-testid="unsaved-marks"]');
    assert(!(await s.exec("return !!document.querySelector('[data-testid=unsaved-save]')")), "no Save copy for marks only");
    await s.click('[data-testid="unsaved-cancel"]');

    // 6. Review and apply.
    await s.click('[data-testid="redaction-marks"]');
    await s.waitFor('[data-testid="redact-preview"] li');
    const preview = await s.exec("return document.querySelector('[data-testid=redact-preview]').textContent");
    for (const want of ["SECRET-4711", "PWHIDE", "1 image", "Page 2: “Page two visible"]) assert(preview.includes(want), `preview lists ${want}: ${preview}`);
    assert((await s.exec("return document.querySelector('#redact-output').value")).endsWith("redaction-redacted.pdf"), "default output name");
    await axeCheck(s, "redact-dialog");
    writeFileSync(join(ART, "redaction-dialog.png"), await s.screenshot());
    await s.type("#redact-output", out);
    await s.click('[data-testid="redact-apply"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved redacted.pdf")), "saved notice", 30000);
    await s.waitUntil(async () => (await annotCount(s, "redact")) === 0 && (await marks()) === "", "marks used up");
    await s.waitUntil(async () => (await s.exec("return [...document.querySelectorAll('.tab-label .name')].map(n => n.textContent)")).includes("redacted.pdf"), "redacted copy opened for inspection");
  });
  assert(existsSync(out), "redacted copy exists");
  assert(sha(src) === before, "source PDF unchanged");
  const raw = readFileSync(out).toString("latin1");
  for (const gone of ["SECRET", "PWHIDE", "PWFORMX"]) assert(!raw.includes(gone), `${gone} not in the file bytes`);
  if (CLI) {
    const t = JSON.parse(execFileSync(CLI, ["--json", "text", "--pages", "1", out], { encoding: "utf8" })).result.data.pages[0].text;
    assert(!t.includes("SECRET") && !t.includes("PWHIDE") && t.includes("is closed") && t.includes("Public footer text"), `independent text check: ${t}`);
    const t2 = JSON.parse(execFileSync(CLI, ["--json", "text", "--pages", "2", out], { encoding: "utf8" })).result.data.pages[0].text;
    assert(t2.trim() === "", `whole-page redaction leaves no text on page 2: ${t2}`);
  }
});

await test("mark patterns for redaction: find sensitive text, untick one match, mark and apply; values gone, the unticked one kept", async () => {
  const src = join(FIX, "sensitive-info.pdf");
  const out = join(work, "sensitive-redacted.pdf");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Mark patterns");
    await s.click('[role="option"][data-command="redact.markPatterns"]');
    await s.waitFor('[data-testid="patterns-find"]');
    await s.click('[role="dialog"] input[data-kind="phone"]'); // leave phone numbers out
    await s.click('[data-testid="patterns-find"]');
    await s.waitUntil(async () => (await s.text('[data-testid="patterns-summary"]')) === "5 matches on 2 pages", "matches found");
    const rows = await s.exec("return [...document.querySelectorAll('[data-testid=patterns-hits] li')].map(l => l.textContent)");
    assert(rows.some((r) => r.includes("E-mail") && r.includes("ada.lovelace@example.org")) && !rows.some((r) => r.includes("7946")), `rows: ${JSON.stringify(rows)}`);
    const ssn = rows.findIndex((r) => r.includes("123-45-6789"));
    await s.click(`[data-testid="patterns-hits"] li:nth-child(${ssn + 1}) input`);
    await s.waitUntil(async () => (await s.text('[data-testid="patterns-mark"]')) === "Mark 4 for redaction", "one match unticked");
    await axeCheck(s, "redact-patterns-dialog");
    writeFileSync(join(ART, "redact-patterns.png"), await s.screenshot());
    await s.click('[data-testid="patterns-mark"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Marked 4 for redaction")), "marked notice");
    await s.waitUntil(async () => (await annotCount(s, "redact")) >= 3, "marks drawn on page 1");
    await s.click('[data-testid="redaction-marks"]');
    await s.waitFor("#redact-output");
    await s.type("#redact-output", out);
    await s.click('[data-testid="redact-apply"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved sensitive-redacted.pdf")), "saved notice", 30000);
  });
  assert(existsSync(out), "redacted copy exists");
  if (CLI) {
    const pages = JSON.parse(execFileSync(CLI, ["--json", "text", out], { encoding: "utf8" })).result.data.pages.map((p) => p.text).join("\n");
    for (const gone of ["lovelace@", "4111 1111 1111 1111", "WEST", "grace@"]) assert(!pages.includes(gone), `${gone} removed: ${pages}`);
    for (const kept of ["123-45-6789", "+44 20 7946 0958", "4111 1111 1111 1112"]) assert(pages.includes(kept), `${kept} kept: ${pages}`);
  }
});

await test("accessibility check: problems grouped with text labels, review items listed, page buttons jump", async () => {
  await withApp([join(FIX, "tagged-issues.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Check accessibility");
    await s.click('[role="option"][data-command="file.accessibility"]');
    await s.waitUntil(async () => (await s.text('[data-testid="a11y-summary"]')).includes("to fix"), "report shown");
    const fix = await s.text('[data-testid="a11y-group-fail"]');
    for (const want of ["Fails", "Document language", "Pictures have alternate text", "Form fields are labelled"]) assert(fix.includes(want), `needs fixing lists ${want}: ${fix}`);
    const warn = await s.text('[data-testid="a11y-group-warning"]');
    assert(warn.includes("Tables have header cells") && warn.includes("Fonts embedded") && warn.includes("Helvetica"), `warnings: ${warn}`);
    assert((await s.text('[data-testid="a11y-group-manual"]')).includes("Reading order"), "manual review items");
    await axeCheck(s, "accessibility-dialog");
    writeFileSync(join(ART, "accessibility-check.png"), await s.screenshot());
    await s.click('[aria-label="Go to page 1 (Pictures have alternate text)"]');
    await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('[data-testid=a11y-summary]')")), "dialog closed after jumping");
  });
});

await test("page labels: roman front matter then prefixed numbers; preview; saved copy shows them in thumbnails", async () => {
  const out = join(work, "five-labelled.pdf");
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Page labels");
    await s.click('[role="option"][data-command="pages.labels"]');
    await s.waitFor('[data-testid="labels-start"]');
    const setSelect = (label, value) =>
      s.exec(`const el = document.querySelector('[aria-label="' + arguments[0] + '"]'); el.value = arguments[1]; el.dispatchEvent(new Event('change', { bubbles: true })); return 1`, label, value);
    await setSelect("Range 1: numbers", "roman_lower");
    await s.click('[data-testid="labels-add"]');
    await s.type('[aria-label="Range 2: from page"]', "3");
    await s.type('[aria-label="Range 2: prefix"]', "Ch-");
    await s.waitUntil(async () => (await s.text('[data-testid="labels-preview"]')) === "Page 1 → i · Page 2 → ii · Page 3 → Ch-1 · Page 4 → Ch-2 · Page 5 → Ch-3", "preview");
    await s.type('[aria-label="Range 2: from page"]', "1");
    await s.waitUntil(async () => (await s.attr('[data-testid="labels-start"]', "title")) === "Range 2 must start after range 1.", "invalid range explained");
    await s.type('[aria-label="Range 2: from page"]', "3");
    await s.type("#labels-output", out);
    await axeCheck(s, "page-labels-dialog");
    writeFileSync(join(ART, "page-labels.png"), await s.screenshot());
    await s.click('[data-testid="labels-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved five-labelled.pdf")), "saved notice");
    await s.click('.notice button[data-action="Open"]');
    await s.waitUntil(async () => (await s.exec("return document.querySelector('.tab-label[aria-current=page] .name')?.textContent")) === "five-labelled.pdf", "copy opened");
    await s.waitUntil(async () => JSON.stringify(await s.exec("return [...document.querySelectorAll('[data-thumb] .num')].map(n => n.textContent)")) === JSON.stringify(["i", "ii", "Ch-1", "Ch-2", "Ch-3"]), "thumbnails show the labels");
  });
  if (CLI) {
    const pages = JSON.parse(execFileSync(CLI, ["--json", "inspect", out], { encoding: "utf8" })).result.data.pages.map((p) => p.label);
    assert(JSON.stringify(pages) === JSON.stringify(["i", "ii", "Ch-1", "Ch-2", "Ch-3"]), `CLI labels ${JSON.stringify(pages)}`);
  }
});

await test("comments panel: move a highlight with the keyboard and by dragging; Escape cancels a move; saved copy has it moved", async () => {
  const src = join(FIX, "hidden-info.pdf");
  const before = sha(src);
  const out = join(work, "comments-moved.pdf");
  const frame = (s) => s.exec("const f=document.querySelector('.annot-frame.active'); if (!f) return null; const r=f.getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height, focused: document.activeElement === f }");
  let k = 1;
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.click("#sb-tab-comments");
    await s.waitFor('[aria-label="Move Highlight on page 1"]');
    // Pixels per point of the page as drawn.
    k = await s.exec("return document.querySelector('.page[data-page=\"0\"]').getBoundingClientRect().width / 612");
    await s.click('[aria-label="Move Highlight on page 1"]');
    await s.waitUntil(async () => (await frame(s))?.focused === true, "the frame is on the page and has the focus");
    const f0 = await frame(s);
    assert((await unsavedText(s)) === "", "picking it up changes nothing yet");
    // Keyboard: Shift+Down twice = 20 pt down.
    await s.keys(["Shift", "ArrowDown"]);
    await s.keys(["Shift", "ArrowDown"]);
    await s.waitUntil(async () => Math.abs((await frame(s)).y - (f0.y + 20 * k)) < 1.5, "moved 20 pt down by keyboard");
    // Drag 60 px to the right (released there).
    const f1 = await frame(s);
    await s.dragAt(f1.x + f1.w / 2, f1.y + f1.h / 2, f1.x + f1.w / 2 + 60, f1.y + f1.h / 2);
    await s.waitUntil(async () => Math.abs((await frame(s)).x - (f1.x + 60)) < 1.5, "dragged 60 px right");
    await s.keys(["Enter"]);
    await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('.annot-frame.active, [data-testid=move-actions]')")), "Enter keeps the move");
    assert(await s.exec("return !!document.querySelector('.annot-frame:not(.active)')"), "the new place stays marked until saved");
    await s.waitUntil(async () => (await unsavedText(s)) === "1 unsaved change", "one unsaved change");
    // Escape puts a move back.
    await s.click('[aria-label="Move Rectangle on page 1"]');
    await s.waitUntil(async () => (await frame(s))?.focused === true, "rectangle picked up");
    await s.keys(["ArrowRight"]);
    await s.waitUntil(async () => (await unsavedText(s)) === "2 unsaved changes", "nudge recorded");
    await s.keys(["Escape"]);
    await s.waitUntil(async () => (await unsavedText(s)) === "1 unsaved change", "Escape cancelled the rectangle's move");
    assert(await s.exec("return !document.querySelector('[role=dialog]')"), "Escape did not open or close anything else");
    await axeCheck(s, "comments-moving");
    writeFileSync(join(ART, "comments-moved.png"), await s.screenshot());
    await s.keys(["Control", "s"]);
    await s.waitFor("#output");
    await s.type("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved comments-moved.pdf")), "saved notice");
    await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('.annot-frame')")), "frames gone after saving");
  });
  assert(sha(src) === before, "source unchanged");
  if (CLI) {
    const rects = (f) => Object.fromEntries(JSON.parse(execFileSync(CLI, ["--json", "annotations", f], { encoding: "utf8" })).result.data.annotations.map((a) => [a.subtype, a.rect]));
    const [a, b] = [rects(src), rects(out)];
    const dx = b.Highlight.left - a.Highlight.left;
    const dy = b.Highlight.bottom - a.Highlight.bottom;
    assert(Math.abs(dy + 20) < 0.05 && Math.abs(dx - 60 / k) < 0.6, `highlight moved by (${dx}, ${dy})`);
    assert(JSON.stringify(b.Square) === JSON.stringify(a.Square), "rectangle not moved");
  }
});

await test("comments panel: resize a rectangle by its corner and with Ctrl+arrows; highlights offer no handles; saved copy has the new box", async () => {
  const src = join(FIX, "hidden-info.pdf");
  const out = join(work, "comments-resized.pdf");
  const frame = (s) => s.exec("const f=document.querySelector('.annot-frame.active'); if (!f) return null; const r=f.getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height, handles: f.querySelectorAll('[data-corner]').length }");
  let k = 1;
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.click("#sb-tab-comments");
    await s.waitFor('[aria-label="Move Highlight on page 1"]');
    k = await s.exec("return document.querySelectorAll('.page')[0].getBoundingClientRect().width / 612");
    // A highlight follows its text: it moves but has no resize handles.
    await s.click('[aria-label="Move Highlight on page 1"]');
    await s.waitUntil(async () => (await frame(s))?.handles === 0, "no handles on a highlight");
    await s.keys(["Control", "ArrowRight"]);
    assert((await unsavedText(s)) === "", "Ctrl+arrow does not resize a highlight");
    await s.keys(["Escape"]);
    await s.click('[aria-label="Move Rectangle on page 1"]');
    await s.waitUntil(async () => (await frame(s))?.handles === 4, "four corner handles on the rectangle");
    const f0 = await frame(s);
    // Bottom-right corner 40 px right and 30 px down.
    const se = await s.exec("const r=document.querySelector('.annot-frame.active [data-corner=se]').getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]");
    await s.dragAt(se[0], se[1], se[0] + 40, se[1] + 30);
    await s.waitUntil(async () => { const f = await frame(s); return Math.abs(f.w - (f0.w + 40)) < 1.5 && Math.abs(f.h - (f0.h + 30)) < 1.5 && Math.abs(f.x - f0.x) < 1; }, "corner drag resized the frame, top-left kept");
    // Ctrl+Shift+Right: 10 pt wider.
    await s.keys(["Control", "Shift", "ArrowRight"]);
    await s.waitUntil(async () => Math.abs((await frame(s)).w - (f0.w + 40 + 10 * k)) < 1.5, "10 pt wider from the keyboard");
    await s.keys(["Enter"]);
    await s.waitUntil(async () => (await unsavedText(s)) === "1 unsaved change", "one unsaved change");
    await axeCheck(s, "comments-resizing");
    writeFileSync(join(ART, "comments-resized.png"), await s.screenshot());
    await s.keys(["Control", "s"]);
    await s.waitFor("#output");
    await s.type("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved comments-resized.pdf")), "saved notice");
  });
  if (CLI) {
    const rects = (f) => Object.fromEntries(JSON.parse(execFileSync(CLI, ["--json", "annotations", f], { encoding: "utf8" })).result.data.annotations.map((a) => [a.subtype, a.rect]));
    const [a, b] = [rects(src).Square, rects(out).Square];
    const near = (x, y, tol = 0.6) => Math.abs(x - y) < tol;
    assert(near(b.left, a.left) && near(b.top, a.top) && near(b.right, a.right + 40 / k + 10) && near(b.bottom, a.bottom - 30 / k), `rectangle ${JSON.stringify(a)} → ${JSON.stringify(b)}`);
  }
});

await test("comments panel: delete a note and change a highlight's comment; closing asks; saved copy verified", async () => {
  const src = join(FIX, "hidden-info.pdf");
  const before = sha(src);
  const out = join(work, "comments-edited.pdf");
  const items = (s) => s.exec("return [...document.querySelectorAll('.comments li')].map(li => li.textContent)");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.click("#sb-tab-comments");
    await s.waitUntil(async () => (await items(s)).length === 4, "four comments listed");
    assert((await items(s))[0].includes("Note") && (await items(s))[0].includes("Reviewer note PWCOMMENT"), `first: ${(await items(s))[0]}`);
    await s.click('[aria-label="Delete Note on page 1"]');
    await s.waitUntil(async () => (await items(s))[0].includes("Removed when you save a copy"), "pending delete shown");
    await s.click('[aria-label="Edit comment of Highlight on page 1"]');
    await s.type('[aria-label="Comment for Highlight on page 1"]', "Checked by QA");
    await s.click('[data-testid="comment-apply"]');
    await s.waitUntil(async () => (await items(s))[1].includes("Checked by QA") && (await items(s))[1].includes("Changed"), "pending text shown");
    // Recolour the rectangle (the native colour picker cannot be driven: set its value as the browser does).
    await s.exec(`const i=document.querySelector('[aria-label="Colour of Rectangle on page 1"]');
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(i, '#0000ff');
      i.dispatchEvent(new Event('input', { bubbles: true })); return 1`);
    await s.waitUntil(async () => (await unsavedText(s)) === "3 unsaved changes", "three unsaved changes");
    await axeCheck(s, "comments-panel");
    writeFileSync(join(ART, "comments-panel.png"), await s.screenshot());
    // Closing the tab asks first.
    await s.keys(["Control", "w"]);
    await s.waitFor('[data-testid="unsaved-cancel"]');
    await s.click('[data-testid="unsaved-cancel"]');
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "s"]);
    await s.waitFor("#output");
    await s.type("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Saved comments-edited.pdf")), "saved notice");
    await s.waitUntil(async () => (await s.exec("return document.querySelector('.tab-label[aria-current=page] .name')?.textContent")) === "comments-edited.pdf", "copy opened");
    await s.click("#sb-tab-comments");
    await s.waitUntil(async () => {
      const it = await items(s);
      return it.length === 3 && it[0].includes("Checked by QA") && !it.join(" ").includes("PWCOMMENT");
    }, "copy lists the changed comments");
    assert((await unsavedText(s)) === "", "nothing unsaved after saving");
  });
  assert(sha(src) === before, "source unchanged");
  if (CLI) {
    const list = execFileSync(CLI, ["annotations", out], { encoding: "utf8" });
    assert(!list.includes("PWCOMMENT") && list.includes("Checked by QA") && list.includes("3 annotation(s)"), `CLI listing:\n${list}`);
    const listed = JSON.parse(execFileSync(CLI, ["--json", "annotations", out], { encoding: "utf8" })).result.data.annotations;
    const square = listed.find((x) => x.subtype === "Square");
    assert(square && square.color === "#0000ff", `rectangle recoloured: ${JSON.stringify(square)}`);
  }
});

await test("batch translation: two English files into German, each named -de.pdf, German text inside", async () => {
  const dir = mkdtempSync(join(work, "batch-tr-"));
  const outDir = join(dir, "out");
  mkdirSync(outDir);
  await withApp([join(FIX, "article.pdf"), join(FIX, "five-pages.pdf")], async (s) => {
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('.tab-label').length")) === 2, "two tabs");
    await pageLoaded(s, 0);
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Process several");
    await s.click('[role="option"][data-command="file.batch"]');
    await s.waitFor('[data-testid="batch-start"]');
    await s.click('[data-testid="batch-add-open"]');
    await s.type("#batch-folder", outDir);
    await s.exec("const el = document.querySelector('#batch-action'); el.value = 'translate'; el.dispatchEvent(new Event('change', { bubbles: true })); return 1");
    await s.waitFor("#batch-target");
    await s.waitUntil(async () => !(await s.exec("return document.querySelector('#batch-target').disabled")), "languages read");
    await s.exec("const el = document.querySelector('#batch-target'); el.value = 'de'; el.dispatchEvent(new Event('change', { bubbles: true })); return 1");
    const blocked = await s.attr('[data-testid="batch-start"]', "title");
    if (blocked) throw new Skip(`translation models not installed here: ${blocked}`);
    await axeCheck(s, "batch-translate");
    await s.click('[data-testid="batch-start"]');
    await s.waitUntil(async () => /2 done|failed/.test(await s.text('[data-testid="batch-summary"]')), "results", 600000);
    const rows = await s.exec("return [...document.querySelectorAll('[data-testid=batch-results] li')].map(l => l.dataset.state + '|' + l.textContent)");
    assert(rows.every((r) => r.startsWith("done|")), `rows: ${JSON.stringify(rows)}`);
  });
  for (const name of ["article-de.pdf", "five-pages-de.pdf"]) {
    const out = join(outDir, name);
    assert(existsSync(out), `${name} written`);
    if (CLI) {
      const text = JSON.parse(execFileSync(CLI, ["--json", "text", out], { encoding: "utf8" })).result.data.pages.map((p) => p.text).join(" ");
      assert(/\b(der|die|das|und|ist)\b/i.test(text), `${name} is German: ${text.slice(0, 160)}`);
    }
  }
});

await test("batch: reduce size of the open files into a folder, again with existing results kept, then an OCR batch stopped part-way", async () => {
  const dir = mkdtempSync(join(work, "batch-"));
  const scans = [1, 2, 3].map((i) => {
    const p = join(dir, `scan${i}.pdf`);
    copyFileSync(join(FIX, "scanned.pdf"), p);
    return p;
  });
  const outDir = join(dir, "out");
  mkdirSync(outDir);
  const openBatch = async (s) => {
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Process several");
    await s.click('[role="option"][data-command="file.batch"]');
    await s.waitFor('[data-testid="batch-start"]');
  };
  const setAction = (s, v) => s.exec("const el = document.querySelector('#batch-action'); el.value = arguments[0]; el.dispatchEvent(new Event('change', { bubbles: true })); return 1", v);
  await withApp([join(FIX, "uncompressed-report.pdf"), join(FIX, "five-pages.pdf")], async (s) => {
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('.tab-label').length")) === 2, "two tabs");
    await pageLoaded(s, 0);
    await openBatch(s);
    assert((await s.attr('[data-testid="batch-start"]', "title")) === "Add PDF files first.", "needs files");
    await s.click('[data-testid="batch-add-open"]');
    await s.type("#batch-folder", outDir);
    await setAction(s, "optimize");
    await axeCheck(s, "batch-dialog");
    await s.click('[data-testid="batch-start"]');
    await s.waitUntil(async () => (await s.text('[data-testid="batch-summary"]')) === "2 done (of 2)", "results", 60000);
    const rows = await s.exec("return [...document.querySelectorAll('[data-testid=batch-results] li')].map(l => l.dataset.state + '|' + l.textContent)");
    assert(rows[0].startsWith("done|uncompressed-report.pdf") && rows[0].includes("Saved uncompressed-report-smaller.pdf"), `rows: ${JSON.stringify(rows)}`);
    writeFileSync(join(ART, "batch-results.png"), await s.screenshot());
    await s.click('[data-testid="batch-close"]');
    // The same batch again: existing results are kept and new ones get another name.
    await openBatch(s);
    await s.click('[data-testid="batch-add-open"]');
    await s.type("#batch-folder", outDir);
    await setAction(s, "optimize");
    await s.click('[data-testid="batch-start"]');
    await s.waitUntil(async () => (await s.text('[data-testid="batch-summary"]')) === "2 done (of 2)", "second run", 60000);
    await s.click('[data-testid="batch-close"]');
  });
  const outs = readdirSync(outDir).filter((f) => f.endsWith(".pdf")).sort();
  assert(outs.length === 4 && outs.includes("uncompressed-report-smaller.pdf") && outs.includes("five-pages-smaller.pdf"), `both runs kept: ${JSON.stringify(outs)}`);
  if (CLI) {
    const t = JSON.parse(execFileSync(CLI, ["--json", "text", "--pages", "1", join(outDir, "five-pages-smaller.pdf")], { encoding: "utf8" })).result.data.pages[0].text;
    assert(t.includes("PWMARK001"), "result keeps the text");
  }

  await withApp(scans, async (s) => {
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('.tab-label').length")) === 3, "three tabs");
    await pageLoaded(s, 0);
    await openBatch(s);
    await s.click('[data-testid="batch-add-open"]');
    await s.type("#batch-folder", outDir);
    await setAction(s, "ocr");
    await s.click('[data-testid="batch-start"]');
    await s.waitFor('[data-testid="batch-stop"]');
    assert((await s.attr('[data-testid="batch-close"]', "aria-disabled")) === "true", "can't close while running");
    await s.click('[data-testid="batch-stop"]');
    await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('[data-testid=batch-stop]')")), "batch stopped", 60000);
    const sum = await s.text('[data-testid="batch-summary"]');
    assert(/cancelled/.test(sum), `some files were stopped: ${sum}`);
  });
  const leftovers = readdirSync(outDir).filter((f) => f.startsWith(".pw-staged"));
  assert(leftovers.length === 0, `no partial files left: ${leftovers}`);
});

await test("settings ▸ resources: PDF engine and text recognition verified against their checksums, speech voices found; check again", async () => {
  await withApp([], async (s) => {
    await s.keys(["Control", ","]);
    await s.waitFor("#tab-resources");
    await s.click("#tab-resources");
    const states = () => s.exec("return [...document.querySelectorAll('[data-testid=resources-list] li')].map(l => l.dataset.resource + ':' + l.dataset.state)");
    // Office documents are optional (LibreOffice installed separately): ready or not installed.
    await s.waitUntil(async () => /^\["pdf_engine:ready","text_recognition:ready","translation:ready","read_aloud:ready","office_documents:(ready|missing)"\]$/.test(JSON.stringify(await states())), "required ready, optional listed", 240000);
    const text = await s.text('[data-testid="resources-list"]');
    assert(!/\.onnx|\.dll|\.so|onnxruntime|pdfium/i.test(text), `no technical file or model names in the UI: ${text}`);
    await s.click('[data-testid="resources-check"]');
    // Every installed file (about 1 GB with the translation models) is hashed; debug builds are slow at it.
    await s.waitUntil(async () => (await s.text('[data-testid="resources-check"]')) === "Check again", "check finished", 240000);
    await axeCheck(s, "settings-resources");
  });
});

await test("all tools pane: every tool by category, filter, runs the tool, hides and comes back (view.toggleTools); Menu ▸ Check for updates (app.updates)", async () => {
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    const open = () => s.exec("return !!document.querySelector('[data-testid=tools-pane]')");
    if (!(await open())) await s.click('[data-command="view.toggleTools"]');
    await s.waitUntil(open, "tools pane shown");
    const groups = await s.exec("return [...document.querySelectorAll('.tools-group h3')].map(h => h.textContent)");
    assert(groups.length >= 10 && groups.includes("Translate and read aloud") && groups.includes("Organize pages"), `groups: ${groups}`);
    await s.type('[data-testid="tools-filter"]', "to word");
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('.tool-item').length")) === 1, "filter narrows to one tool");
    await s.click('[data-tool="file.toWord"]');
    await s.waitFor("#output");
    assert((await s.exec("return document.querySelector('#output').value")).endsWith(".docx"), "Convert to Word opened from the pane");
    await s.keys(["Escape"]);
    await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('#output')")), "dialog closed");
    await axeCheck(s, "tools-pane");
    await s.click('[data-testid="tools-close"]');
    await s.waitUntil(async () => !(await open()), "pane closed");
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "Shift", "T"]);
    await s.waitUntil(open, "Ctrl+Shift+T shows it again");
    // Settings moved into the Menu (Preferences); updates are there too.
    await s.click('[data-testid="app-menu"]');
    await s.waitFor('[role="menu"]');
    const items = await s.text('[role="menu"]');
    assert(items.includes("Preferences") && items.includes("Check for updates"), `menu: ${items}`);
    await s.click('[role="menuitem"][data-command="app.updates"]');
    await s.waitFor('[data-testid="updates-panel"]');
  });
});

await test("settings ▸ updates: check now reads the release page; the startup check setting is saved", async () => {
  await withApp([], async (s) => {
    await s.keys(["Control", ","]);
    await s.waitFor("#tab-updates");
    await s.click("#tab-updates");
    await s.waitUntil(async () => /PageWeave \d+\.\d+\.\d+/.test(await s.text('[data-testid="updates-panel"] strong')), "version shown");
    await s.click('[data-testid="updates-check"]');
    // Up to date, or (when this build is older than the newest release) an offer; never an error.
    await s.waitUntil(async () => /up to date|is available/.test(await s.text('[data-testid="updates-status"]')), "check answered", 60000);
    assert(!(await s.exec("return !!document.querySelector('[data-testid=updates-status] [role=alert]')")), "no error");
    assert((await s.exec("return document.querySelector('[data-testid=updates-auto]').checked")) === true, "on by default");
    await s.click('[data-testid="updates-auto"]');
    await axeCheck(s, "settings-updates");
    await s.click('[role="dialog"] .btn.primary');
    await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('#tab-updates')")), "saved and closed");
    await s.keys(["Control", ","]);
    await s.waitFor("#tab-updates");
    await s.click("#tab-updates");
    await s.waitFor('[data-testid="updates-auto"]');
    assert((await s.exec("return document.querySelector('[data-testid=updates-auto]').checked")) === false, "setting kept");
    await s.click('[data-testid="updates-auto"]');
    await s.click('[role="dialog"] .btn.primary');
  });
});

await test("engine crash on a damaged file fails the job with a reason; the app stays open and keeps working", async () => {
  const src = join(FIX, "..", "hostile", "redact-generate-content-crash.pdf");
  const out = join(work, "crash-redacted.pdf");
  await withApp([src], async (s) => {
    await pageLoaded(s, 0);
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", "k"]);
    await s.type('[role="dialog"] input', "Mark current page");
    await s.click('[role="option"][data-command="redact.markPage"]');
    await s.waitUntil(async () => (await annotCount(s, "redact")) === 1, "page marked");
    await s.click('[data-testid="redaction-marks"]');
    await s.waitFor("#redact-output");
    await s.type("#redact-output", out);
    await s.click('[data-testid="redact-apply"]');
    await s.waitUntil(async () => (await s.exec("return document.querySelector('[role=dialog] [role=alert]')?.textContent ?? ''")).includes("stopped unexpectedly"), "crash reported in the dialog", 60000);
    assert(!existsSync(out), "nothing saved");
    // Still alive: close the dialog and use the app.
    await s.keys(["Escape"]);
    await s.waitUntil(async () => !(await s.exec("return !!document.querySelector('[role=dialog]')")), "dialog closed");
    await s.keys(["Control", "d"]);
    await s.waitUntil(async () => await s.exec("return !!document.querySelector('[role=dialog]')"), "app still responds");
  });
});

await test("viewer: an engine crash while showing a page fails only that page; other pages, Retry and the app keep working", async () => {
  // Debug builds honour PW_READER_TEST in the reader processes; release builds ignore it.
  if (!/[\\/]debug[\\/]/.test(APP)) throw new Skip("needs a debug build (the crash hook is compiled out of release builds)");
  // The test driver starts the app with its own environment, so the hook goes in a file.
  const hook = join(tmpdir(), "pageweave-reader-test");
  writeFileSync(hook, "abort:2");
  try {
    await withApp([join(FIX, "five-pages.pdf")], async (s) => {
      await pageLoaded(s, 0);
      await s.exec("document.querySelector('[data-page=\"2\"]').scrollIntoView()");
      await s.waitUntil(async () => (await s.exec("return document.querySelector('[data-page=\"2\"] .page-error')?.textContent ?? ''")).includes("Page 3 could not be displayed"), "page 3 fails with a reason", 30000);
      await pageLoaded(s, 1);
      await s.exec("document.querySelector('[data-page=\"3\"]').scrollIntoView()");
      await pageLoaded(s, 3);
      // Retry answers at once with the same failure instead of crashing another reader.
      await s.exec("document.querySelector('[data-page=\"2\"]').scrollIntoView()");
      await s.exec("document.querySelector('[data-page=\"2\"] .page-error button').click()");
      await s.waitUntil(async () => (await s.exec("return !!document.querySelector('[data-page=\"2\"] .page-error')")), "still the same reason");
      // The app keeps working: search finds text on another page.
      await s.click('[data-testid="viewer"]');
      await s.keys(["Control", "f"]);
      await s.waitFor('input[aria-label="Find in document"]');
      await s.type('input[aria-label="Find in document"]', "PWMARK004");
      await s.keys(["Enter"]);
      await s.waitUntil(async () => (await s.text(".searchbar .count")) === "1 of 1", "search works", 20000);
    });
  } finally {
    rmSync(hook, { force: true });
  }
});

/* ------------------------------------------------------------------ */
const summary = {
  app: APP,
  when: new Date().toISOString(),
  passed: results.filter((r) => r.status === "PASS").length,
  failed: results.filter((r) => r.status === "FAIL").length,
  skipped: results.filter((r) => r.status === "SKIP").length,
  launch_retries: launchRetries,
  results,
};
writeFileSync(join(ART, "e2e-results.json"), JSON.stringify(summary, null, 2));
console.log(`\n${summary.passed} passed, ${summary.failed} failed, ${summary.skipped} skipped, ${launchRetries.length} app launch retr${launchRetries.length === 1 ? "y" : "ies"}`);
process.exit(summary.failed ? 1 : 0);
