// Real-application end-to-end tests (QUALITY_GATES Gate 3). Drives the built PageWeave binary through
// tauri-driver with real mouse/keyboard input, the real engine and real files.
//
// Usage: node e2e/run.mjs <app-binary> <artifact-dir>   (run via e2e/run.sh, which starts the display + driver)

import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
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

async function withApp(args, fn) {
  const t0 = Date.now();
  let s;
  try {
    s = await Session.create(DRIVER, APP, appArgs(args));
  } catch (e) {
    // Seen on CI: GTK occasionally fails to connect to the virtual display at process start, and the
    // driver then waits out its timeout. Retry the launch once and record it; a second failure is real.
    launchRetries.push({ test: currentTest, error: String(e?.message ?? e).slice(0, 300) });
    console.log(`  note: app launch failed (${String(e?.message ?? e).slice(0, 160)}); retrying the launch once`);
    try {
      s = await Session.create(DRIVER, APP, appArgs(args));
    } catch (e2) {
      launchFailuresInARow++;
      throw new LaunchFailed(`the app could not be started under WebDriver twice: ${String(e2?.message ?? e2).slice(0, 200)}`);
    }
  }
  launchFailuresInARow = 0;
  phase.create += Date.now() - t0;
  try {
    await s.waitFor(args.length ? '.tab-label' : ".home", { timeout: 20000 });
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

// E2E_ONLY=<substring> runs only matching scenarios (local iteration; CI always runs everything).
const ONLY = process.env.E2E_ONLY;

/** Thrown by a scenario that cannot run on this platform; reported as SKIP, never as PASS. */
class Skip extends Error {}

async function test(name, fn) {
  if (ONLY && !name.includes(ONLY)) return;
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
    const title = await s.attr('[data-command="view.zoomIn"]', "title");
    assert((await s.attr('[data-command="view.zoomIn"]', "aria-disabled")) === "true", "zoom disabled");
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
    await s.type('[data-bookmark-page="0"]', "9");
    await s.waitUntil(async () => ((await s.attr('[data-testid="bookmarks-save"]', "title")) ?? "").includes("from 1 to 5"), "page range explained");
    await s.type('[data-bookmark-page="0"]', "3");
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
    assert((await s.exec("return document.querySelector('#pdf-password')")) === null, "no password asked");
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
