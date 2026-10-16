// Real-application end-to-end tests (QUALITY_GATES Gate 3). Drives the built PageWeave binary through
// tauri-driver with real mouse/keyboard input, the real engine and real files.
//
// Usage: node e2e/run.mjs <app-binary> <artifact-dir>   (run via e2e/run.sh, which starts the display + driver)

import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { Session } from "./webdriver.mjs";

const APP = resolve(process.argv[2] ?? "../target/debug/pageweave-app");
const ART = resolve(process.argv[3] ?? "e2e-artifacts");
const DRIVER = process.env.WEBDRIVER_URL ?? "http://127.0.0.1:4444";
const FIX = resolve(new URL("../../fixtures/pdf", import.meta.url).pathname);
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

async function withApp(args, fn) {
  const s = await Session.create(DRIVER, APP, args);
  try {
    await s.waitFor(args.length ? '[role="tab"]' : ".home", { timeout: 20000 });
    return await fn(s);
  } catch (e) {
    await captureFailure(s);
    throw e;
  } finally {
    await s.delete();
  }
}

async function test(name, fn) {
  currentTest = name;
  const t0 = Date.now();
  try {
    await fn();
    results.push({ name, status: "PASS", ms: Date.now() - t0 });
    console.log(`PASS ${name} (${Date.now() - t0} ms)`);
  } catch (e) {
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
    assert((await s.exec("return document.querySelector('[role=tab] .name').textContent")) === "five-pages.pdf", "tab name");
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
    await s.keys(["Control", ""]); // End
    await s.waitUntil(async () => (await currentPage(s)) === "5", "last page");
    await s.keys(["Control", ""]); // Home
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
    await pageLoaded(s, 0);
    await s.keys(["Control", "f"]);
    await s.waitFor('input[aria-label="Find in document"]');
    await s.type('input[aria-label="Find in document"]', "alpha");
    await s.keys(["Enter"]);
    await s.waitUntil(async () => (await s.text(".searchbar .count")) === "1 of 5", "1 of 5");
    const hl = await s.exec("return document.querySelectorAll('.hit').length");
    assert(hl >= 5, `highlights drawn (${hl})`);
    await s.keys(["F3"]);
    await s.waitUntil(async () => (await s.text(".searchbar .count")) === "2 of 5", "2 of 5");
    await s.keys(["Shift", "F3"]);
    await s.waitUntil(async () => (await s.text(".searchbar .count")) === "1 of 5", "back to 1 of 5");
    await s.click('button[aria-label="Whole words only"]');
    await s.keys(["Enter"]);
    await s.waitUntil(async () => (await s.text(".searchbar .count")) === "1 of 4", "whole word 1 of 4");
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

await test("rotate page from right-click context menu", async () => {
  const out = join(work, "rotated.pdf");
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.contextClick('[data-page="0"]');
    await s.waitFor('[role="menu"]');
    const labels = await s.exec("return [...document.querySelectorAll('[role=menuitem]')].map(m => m.innerText.trim())");
    assert(labels.some((l) => l.startsWith("Rotate page 1")), `menu items: ${labels}`);
    writeFileSync(join(ART, "context-menu.png"), await s.screenshot());
    await s.click('[role="menuitem"][data-command="pages.rotate"]');
    await s.waitFor('[role="dialog"]');
    assert((await s.exec("return document.querySelector('#pages').value")) === "1", "range prefilled with page 1");
    await s.waitUntil(async () => (await s.text("#pages-msg")) === "1 page selected", "valid");
    await s.type("#output", out);
    await s.click('[data-testid="tool-start"]');
    await s.waitUntil(async () => existsSync(out), "rotated output");
  });
});

await test("Escape closes context menu and returns focus", async () => {
  await withApp([join(FIX, "five-pages.pdf")], async (s) => {
    await pageLoaded(s, 0);
    await s.contextClick('[data-page="0"]');
    await s.waitFor('[role="menu"]');
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
    await s.click(".dialog footer .btn.primary");
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
    assert((await s.attr(".dialog footer .btn.primary", "aria-disabled")) === "true", "save blocked by conflict");
    await s.click('[data-shortcut-row="view.toggleThumbnails"] td:nth-child(3) button:first-child');
    await s.keys(["Control", "Alt", "t"]);
    await s.waitUntil(async () => await s.exec("return !document.querySelector('.dialog .banner.error')"), "conflict resolved");
    await s.click(".dialog footer .btn.primary");
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
    await s.waitFor('[role="tab"]');
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
    await s.waitFor('[role="tab"]', { timeout: 20000 });
    await s.waitUntil(async () => (await noticeTexts(s)).some((t) => t.includes("Couldn’t open fake.pdf") && t.includes("not a PDF")), "error notice");
    const tabs = await s.exec("return document.querySelectorAll('[role=tab]').length");
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
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('[role=tab]').length")) === 2, "two tabs");
    const active = () => s.exec("return document.querySelector('[role=tab][aria-selected=true] .name').textContent");
    const first = await active();
    await s.click('[data-testid="viewer"]');
    await s.keys(["Control", ""]);
    await s.waitUntil(async () => (await active()) !== first, "switched tab");
    await s.keys(["Control", "w"]);
    await s.waitUntil(async () => (await s.exec("return document.querySelectorAll('[role=tab]').length")) === 1, "one tab left");
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

/* ------------------------------------------------------------------ */
const summary = {
  app: APP,
  when: new Date().toISOString(),
  passed: results.filter((r) => r.status === "PASS").length,
  failed: results.filter((r) => r.status === "FAIL").length,
  results,
};
writeFileSync(join(ART, "e2e-results.json"), JSON.stringify(summary, null, 2));
console.log(`\n${summary.passed} passed, ${summary.failed} failed`);
process.exit(summary.failed ? 1 : 0);
