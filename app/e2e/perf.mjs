// Performance measurements of the real app and the command-line engine on this machine.
// Run under the E2E harness (virtual display, isolated profile):
//   E2E_SCRIPT=e2e/perf.mjs PAGEWEAVE_CLI=$PWD/target/debug/pageweave app/e2e/run.sh <app> <artifact-dir>
// Writes perf-results.json into the artifact folder and prints a summary. Measures, never asserts:
// budgets are set from these numbers (docs/benchmarks/PERFORMANCE_BASELINE.md).
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { cpus, totalmem } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { Session } from "./webdriver.mjs";

const [, , APP, ART] = process.argv;
const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const FIX = join(ROOT, "fixtures/pdf");
const LONG = join(ROOT, "target/corpus/made/rl-long.pdf");
const CLI = process.env.PAGEWEAVE_CLI;
const DRIVER = process.env.WEBDRIVER_URL ?? "http://127.0.0.1:4444";
const RUNS = Number(process.env.PERF_RUNS ?? 3);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const median = (v) => [...v].sort((a, b) => a - b)[Math.floor(v.length / 2)];
const pct = (v, p) => [...v].sort((a, b) => a - b)[Math.min(v.length - 1, Math.floor(v.length * p))];

/** Resident memory (MiB) of the app and every process started under it (web views, workers),
 * in total and by program. */
function treeRss() {
  const procs = readdirSync("/proc").filter((d) => /^\d+$/.test(d));
  const info = new Map();
  for (const pid of procs) {
    try {
      const stat = readFileSync(`/proc/${pid}/stat`, "utf8");
      const ppid = Number(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[1]);
      const rss = Number(/VmRSS:\s+(\d+)/.exec(readFileSync(`/proc/${pid}/status`, "utf8"))?.[1] ?? 0);
      const exe = readFileSync(`/proc/${pid}/cmdline`, "utf8").split("\0")[0];
      info.set(pid, { ppid: String(ppid), rss, exe });
    } catch {
      // The process ended while being read.
    }
  }
  const roots = [...info].filter(([, p]) => p.exe.endsWith("pageweave-app")).map(([pid]) => pid);
  const inTree = (pid) => {
    for (let p = pid, n = 0; p && n < 64; p = info.get(p)?.ppid, n++) if (roots.includes(p)) return true;
    return false;
  };
  const by = {};
  let kib = 0;
  for (const [pid, p] of info) {
    if (!inTree(pid)) continue;
    kib += p.rss;
    const name = p.exe.split("/").pop();
    by[name] = (by[name] ?? 0) + Math.round(p.rss / 1024);
  }
  return { total: Math.round(kib / 1024), by };
}
const treeRssMiB = () => treeRss().total;

async function waitFor(fn, what, timeout = 60000) {
  const t0 = Date.now();
  for (;;) {
    const v = await fn().catch(() => null);
    if (v) return v;
    if (Date.now() - t0 > timeout) throw new Error(`timed out: ${what}`);
    await sleep(25);
  }
}

const pageShown = (s, i) => s.exec(`const img = document.querySelector('.page[data-page="${i}"] img'); return !!(img && img.complete && img.naturalWidth > 0)`);

/** Startup marks the host logged this run (ms since the host process started). */
async function marks(s) {
  const dir = await s.execAsync("const done = arguments[arguments.length - 1]; window.__TAURI_INTERNALS__.invoke('log_dir').then(done, () => done(null));");
  const out = {};
  if (!dir || !existsSync(dir)) return out;
  for (const f of readdirSync(dir)) {
    for (const line of readFileSync(join(dir, f), "utf8").split("\n")) {
      if (!line.includes('"timing"')) continue;
      try {
        const e = JSON.parse(line);
        const m = e.fields?.mark ?? e.mark;
        const ms = e.fields?.since_start_ms ?? e.since_start_ms;
        if (m && ms !== undefined) out[m] = Number(ms);
      } catch {
        // Not a JSON line.
      }
    }
  }
  return out;
}

const results = { machine: { cpus: cpus().length, cpu: cpus()[0]?.model, ram_gib: Math.round(totalmem() / 2 ** 30), platform: process.platform }, startup: [], viewer: null, cli: {} };

// Startup: time to the first page shown, as seen from outside and as logged by the host.
for (let i = 0; i < RUNS; i++) {
  const t0 = Date.now();
  const s = await Session.create(DRIVER, APP, [join(FIX, "article.pdf")]);
  try {
    await waitFor(() => pageShown(s, 0), "first page");
    const seen = Date.now() - t0;
    await sleep(1500);
    const m = await waitFor(async () => { const x = await marks(s); return x.first_page !== undefined ? x : null; }, "startup marks", 10000).catch(() => ({}));
    results.startup.push({ run: i + 1, first_page_seen_ms: seen, ui_ready_ms: m.ui_ready ?? null, first_page_ms: m.first_page ?? null, idle_rss_mib: treeRssMiB() });
  } finally {
    await s.delete();
  }
  await sleep(1000);
}

// Viewer: a 300-page document — open, scroll through, jump, memory before and after.
if (existsSync(LONG)) {
  const t0 = Date.now();
  const s = await Session.create(DRIVER, APP, [LONG]);
  try {
    await waitFor(() => pageShown(s, 0), "first page of the long document");
    const opened = Date.now() - t0;
    await sleep(2000);
    const rssOpen = treeRssMiB();
    // Scroll for 4 s at two speeds (px per animation frame) and record frame times.
    const scroll = (px) =>
      s.execAsync(`const done = arguments[arguments.length - 1];
      const v = document.querySelector('[data-testid=viewer]'); const t = [];
      let last = performance.now(); const end = last + 4000;
      function step(now) { t.push(now - last); last = now; v.scrollTop += ${px}; if (now < end) requestAnimationFrame(step); else done(t.slice(1)); }
      requestAnimationFrame(step);`);
    const stats = (f) => ({ frames: f.length, median_ms: Math.round(median(f) * 10) / 10, p95_ms: Math.round(pct(f, 0.95) * 10) / 10, over_50ms: f.filter((x) => x > 50).length });
    const fast = await scroll(120);
    await sleep(1500);
    await s.exec("document.querySelector('[data-testid=viewer]').scrollTop = 0; return 1");
    await sleep(2000);
    const steady = await scroll(30);
    await sleep(1500);
    const frames = fast;
    // Jump to page 250 and time until its picture is shown.
    const j0 = Date.now();
    await s.exec("document.querySelector('[data-testid=viewer]').focus()");
    await s.exec(`const v = document.querySelector('[data-testid=viewer]'); const p = document.querySelector('.page[data-page="249"]'); v.scrollTop = p.offsetTop;`);
    await waitFor(() => pageShown(s, 249), "page 250 shown", 30000);
    const jump = Date.now() - j0;
    await sleep(2000);
    const rssAfter = treeRssMiB();
    const rendered = await s.exec("return document.querySelectorAll('.page img').length");
    results.viewer = {
      pages: 300,
      open_to_first_page_ms: opened,
      rss_open_mib: rssOpen,
      rss_after_scroll_mib: rssAfter,
      page_images_kept: rendered,
      scroll_120px_per_frame: stats(frames),
      scroll_30px_per_frame: stats(steady),
      rss_by_program_mib: treeRss().by,
      jump_to_page_250_ms: jump,
    };
  } finally {
    await s.delete();
  }
}

// Engine throughput through the command line (one run each; wall time).
if (CLI) {
  const time = (name, args) => {
    const t0 = process.hrtime.bigint();
    execFileSync(CLI, args, { stdio: "ignore" });
    results.cli[name] = Math.round(Number(process.hrtime.bigint() - t0) / 1e6);
  };
  const out = join(ART, "perf-out");
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });
  time("inspect_300_pages_ms", ["inspect", LONG]);
  time("text_300_pages_ms", ["text", LONG, "-o", join(out, "long.txt")]);
  time("render_page_2x_ms", ["render", join(FIX, "article.pdf"), "--page", "1", "--scale", "2", "-o", join(out, "p.png")]);
  time("search_300_pages_ms", ["search", LONG, "the"]);
  time("to_docx_3_pages_ms", ["to-docx", join(FIX, "article.pdf"), "-o", join(out, "a.docx")]);
  time("ocr_3_scanned_pages_ms", ["ocr", join(FIX, "scanned.pdf"), "-o", join(out, "s.pdf")]);
  time("translate_1_page_en_de_ms", ["translate", join(FIX, "five-pages.pdf"), "--pages", "1", "--to", "de", "-o", join(out, "t.txt")]);
  time("design_rebuild_page_ms", ["design", join(out, "p.png"), "-o", join(out, "p.pwdesign")]);
}

writeFileSync(join(ART, "perf-results.json"), JSON.stringify(results, null, 1));
console.log(JSON.stringify(results, null, 1));
