// Screenshots of the real app for design review: node e2e/shot.mjs <app> <out-dir> <name> [files…]
// Run through e2e/run.sh with E2E_SCRIPT=e2e/shot.mjs (virtual display, isolated settings).
// SHOT_SIZE: optional window size ("900x640"). SHOT_JS: optional JavaScript run in the window before the screenshot (opens a dialog, etc.).
import { writeFileSync, mkdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { Session } from "./webdriver.mjs";

const [, , app, out, name = "shot", ...files] = process.argv;
mkdirSync(resolve(out), { recursive: true });
const s = await Session.create(process.env.WEBDRIVER_URL ?? "http://127.0.0.1:4444", resolve(app), files.map((f) => resolve(f)));
try {
  // SHOT_SIZE: optional window size, e.g. "900x640".
  if (process.env.SHOT_SIZE) {
    const [w, h] = process.env.SHOT_SIZE.split("x").map(Number);
    await s.setWindowRect(w, h);
  }
  await new Promise((r) => setTimeout(r, 5000));
  if (process.env.SHOT_JS) {
    await s.exec(process.env.SHOT_JS);
    await new Promise((r) => setTimeout(r, 1500));
  }
  writeFileSync(join(resolve(out), `${name}.png`), await s.screenshot());
  console.log("saved", join(resolve(out), `${name}.png`));
} finally {
  await s.delete();
}
