// Real-application end-to-end tests (QUALITY_GATES Gate 3). Drives the built PageWeave binary through
// tauri-driver with real mouse/keyboard input, the real engine and real files.
//
// Usage: node e2e/run.mjs <app-binary> <artifact-dir>   (run via e2e/run.sh, which starts the display + driver)

import { execFileSync } from "node:child_process";
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
}

const phase = { create: 0, teardown: 0 };
/** App launches that failed before a session existed and were retried (reported in the summary). */
const launchRetries = [];
/** Consecutive scenarios whose app could not be launched at all; the suite stops after 3. */
let launchFailuresInARow = 0;
class LaunchFailed extends Error {}

async function withApp(args, fn) {
  const t0 = Date.now();
  let s;
  try {
    s = await Session.create(DRIVER, APP, args);
  } catch (e) {
    // Seen on CI: GTK occasionally fails to connect to the virtual display at process start, and the
    // driver then waits out its timeout. Retry the launch once and record it; a second failure is real.
    launchRetries.push({ test: currentTest, error: String(e?.message ?? e).slice(0, 300) });
    console.log(`  note: app launch failed (${String(e?.message ?? e).slice(0, 160)}); retrying the launch once`);
    try {
      s = await Session.create(DRIVER, APP, args);
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
  const s = await Session.create(DRIVER, APP, [join(FIX, "encrypted-rc4-user-password.pdf")]);
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
  const s = await Session.create(DRIVER, APP, [fake, join(FIX, "single-page.pdf")]);
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
  if (process.platform !== "linux" || !process.env.DISPLAY) throw new Skip("needs an X11 display to send a window close request");
  execFileSync("python3", ["-I", fileURLToPath(new URL("./wm_close.py", import.meta.url)), "PageWeave"], { stdio: "inherit" });
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
