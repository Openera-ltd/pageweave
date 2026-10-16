// Minimal W3C WebDriver client (no dependencies) for driving the real PageWeave app via tauri-driver.

const KEYS = {
  Control: "\uE009",
  Shift: "\uE008",
  Alt: "\uE00A",
  Enter: "\uE007",
  Escape: "\uE00C",
  Tab: "\uE004",
  Backspace: "\uE003",
  PageUp: "\uE00E",
  PageDown: "\uE00F",
  End: "\uE010",
  Home: "\uE011",
  ArrowDown: "\uE015",
  F3: "\uE033",
  F10: "\uE03A",
};
export { KEYS };

const ELEMENT = "element-6066-11e4-a52e-4f735466cecf";

export class Session {
  constructor(base, id) {
    this.base = base;
    this.id = id;
  }

  static async create(base, application, args = []) {
    const res = await fetch(`${base}/session`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ capabilities: { alwaysMatch: { "tauri:options": { application, args } } } }),
    });
    const body = await res.json();
    if (!res.ok || !body.value?.sessionId) throw new Error(`session failed: ${JSON.stringify(body)}`);
    return new Session(base, body.value.sessionId);
  }

  async cmd(method, path, data) {
    const res = await fetch(`${this.base}/session/${this.id}${path}`, {
      method,
      headers: { "content-type": "application/json" },
      body: data === undefined ? undefined : JSON.stringify(data),
    });
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(`${method} ${path} → ${res.status} ${JSON.stringify(body.value ?? body).slice(0, 400)}`);
    return body.value;
  }

  async exec(script, ...args) {
    return this.cmd("POST", "/execute/sync", { script, args });
  }

  async find(css) {
    const v = await this.cmd("POST", "/element", { using: "css selector", value: css });
    return v[ELEMENT];
  }

  async findAll(css) {
    const v = await this.cmd("POST", "/elements", { using: "css selector", value: css });
    return v.map((e) => e[ELEMENT]);
  }

  async click(css) {
    const el = await this.waitFor(css);
    await this.cmd("POST", `/element/${el}/click`, {});
  }

  /** Type into a field and verify the field received exactly that text (guards against input-simulation quirks). */
  async type(css, text) {
    const el = await this.waitFor(css);
    await this.cmd("POST", `/element/${el}/clear`, {});
    await this.cmd("POST", `/element/${el}/value`, { text });
    const got = await this.exec("return arguments[0].value", { [ELEMENT]: el });
    if (got !== text) throw new Error(`typed ${JSON.stringify(text)} into ${css} but the field contains ${JSON.stringify(got)} (WebDriver input simulation)`);
  }

  async text(css) {
    const el = await this.waitFor(css);
    return this.cmd("GET", `/element/${el}/text`);
  }

  async attr(css, name) {
    const el = await this.waitFor(css);
    return this.cmd("GET", `/element/${el}/attribute/${name}`);
  }

  /** Press a chord, e.g. keys(["Control", "f"]). Modifiers are held while the last key is pressed. */
  async keys(chord) {
    const vals = chord.map((k) => KEYS[k] ?? k);
    const down = vals.map((v) => ({ type: "keyDown", value: v }));
    const up = [...vals].reverse().map((v) => ({ type: "keyUp", value: v }));
    await this.cmd("POST", "/actions", { actions: [{ type: "key", id: "kb", actions: [...down, ...up] }] });
    await this.cmd("DELETE", "/actions");
  }

  /** Right-click at the centre of an element. */
  async contextClick(css) {
    const el = await this.waitFor(css);
    await this.cmd("POST", "/actions", {
      actions: [
        {
          type: "pointer",
          id: "mouse",
          parameters: { pointerType: "mouse" },
          actions: [
            { type: "pointerMove", origin: { [ELEMENT]: el }, x: 0, y: 0 },
            { type: "pointerDown", button: 2 },
            { type: "pointerUp", button: 2 },
          ],
        },
      ],
    });
    await this.cmd("DELETE", "/actions");
  }

  async screenshot() {
    return Buffer.from(await this.cmd("GET", "/screenshot"), "base64");
  }

  async setWindowRect(width, height) {
    return this.cmd("POST", "/window/rect", { width, height });
  }

  async waitFor(css, { timeout = 10000, visible = true } = {}) {
    const end = Date.now() + timeout;
    let last;
    while (Date.now() < end) {
      try {
        const el = await this.find(css);
        if (!visible) return el;
        const shown = await this.cmd("GET", `/element/${el}/displayed`).catch(() => true);
        if (shown) return el;
      } catch (e) {
        last = e;
      }
      await new Promise((r) => setTimeout(r, 100));
    }
    throw new Error(`timed out waiting for ${css}: ${last?.message ?? "not visible"}`);
  }

  async waitUntil(fn, what, timeout = 15000) {
    const end = Date.now() + timeout;
    let v;
    while (Date.now() < end) {
      v = await fn();
      if (v) return v;
      await new Promise((r) => setTimeout(r, 100));
    }
    throw new Error(`timed out waiting for: ${what} (last value ${JSON.stringify(v)})`);
  }

  async delete() {
    await fetch(`${this.base}/session/${this.id}`, { method: "DELETE" }).catch(() => undefined);
  }
}
