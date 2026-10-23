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
  ArrowLeft: "\uE012",
  ArrowUp: "\uE013",
  ArrowRight: "\uE014",
  ArrowDown: "\uE015",
  Delete: "\uE017",
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
      signal: AbortSignal.timeout(60_000),
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ capabilities: { alwaysMatch: { "tauri:options": { application, args } } } }),
    });
    const body = await res.json();
    if (!res.ok || !body.value?.sessionId) throw new Error(`session failed: ${JSON.stringify(body)}`);
    return new Session(base, body.value.sessionId);
  }

  async cmd(method, path, data) {
    // Every command is bounded so a hung driver or app fails fast instead of stalling CI.
    const res = await fetch(`${this.base}/session/${this.id}${path}`, {
      signal: AbortSignal.timeout(30_000),
      method,
      headers: { "content-type": "application/json" },
      body: data === undefined ? undefined : JSON.stringify(data),
    });
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(`${method} ${path} → ${res.status} ${JSON.stringify(body.value ?? body).slice(0, 400)}`);
    return body.value;
  }

  // Return plain values (booleans, strings, numbers), never DOM elements: an element removed while
  // the driver serializes it fails with "stale element reference" (seen in CI run #73, print).
  async exec(script, ...args) {
    return this.cmd("POST", "/execute/sync", { script, args });
  }

  /** Run an async script; it must call the callback passed as its last argument. */
  async execAsync(script, ...args) {
    return this.cmd("POST", "/execute/async", { script, args });
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
    let got = await this.exec("return arguments[0].value", { [ELEMENT]: el });
    if (got !== text && got.toLowerCase() === text.toLowerCase()) {
      // WebKitWebDriver loses Shift for capitals in element send-keys after earlier pointer actions.
      // Re-type through the Actions API with explicit Shift presses (still real key events).
      await this.cmd("POST", `/element/${el}/clear`, {});
      await this.exec("arguments[0].focus(); return 1", { [ELEMENT]: el });
      const acts = [];
      for (const ch of text) {
        const shifted = ch !== ch.toLowerCase();
        if (shifted) acts.push({ type: "keyDown", value: KEYS.Shift });
        acts.push({ type: "keyDown", value: ch }, { type: "keyUp", value: ch });
        if (shifted) acts.push({ type: "keyUp", value: KEYS.Shift });
      }
      await this.cmd("POST", "/actions", { actions: [{ type: "key", id: "kb", actions: acts }] });
      await this.cmd("DELETE", "/actions");
      got = await this.exec("return arguments[0].value", { [ELEMENT]: el });
    }
    if (got !== text) throw new Error(`typed ${JSON.stringify(text)} into ${css} but the field contains ${JSON.stringify(got)} (WebDriver input simulation)`);
  }

  /** Type into the focused element key by key (newline = Enter), as a user would. */
  async typeKeys(text) {
    const acts = [];
    for (const ch of text) {
      const key = ch === "\n" ? KEYS.Enter : ch;
      const shifted = ch !== "\n" && ch !== ch.toLowerCase();
      if (shifted) acts.push({ type: "keyDown", value: KEYS.Shift });
      acts.push({ type: "keyDown", value: key }, { type: "keyUp", value: key });
      if (shifted) acts.push({ type: "keyUp", value: KEYS.Shift });
    }
    await this.cmd("POST", "/actions", { actions: [{ type: "key", id: "kb", actions: acts }] });
    await this.cmd("DELETE", "/actions");
  }

  /**
   * Replace a field's content as a user does: focus, Ctrl+A, type. For controlled number inputs,
   * where WebDriver's element clear is not seen as an edit by every engine (WebView2 kept "3" → "39").
   */
  async replaceText(css, text) {
    const el = await this.waitFor(css);
    await this.exec("arguments[0].focus(); return 1", { [ELEMENT]: el });
    await this.keys(["Control", "a"]);
    await this.typeKeys(text);
    const got = await this.exec("return arguments[0].value", { [ELEMENT]: el });
    if (got !== text) throw new Error(`replaced ${css} with ${JSON.stringify(text)} but the field contains ${JSON.stringify(got)}`);
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
  async contextClick(css, dx = 0, dy = 0) {
    const el = await this.waitFor(css);
    await this.cmd("POST", "/actions", {
      actions: [
        {
          type: "pointer",
          id: "mouse",
          parameters: { pointerType: "mouse" },
          actions: [
            { type: "pointerMove", origin: { [ELEMENT]: el }, x: Math.round(dx), y: Math.round(dy) },
            { type: "pointerDown", button: 2 },
            { type: "pointerUp", button: 2 },
          ],
        },
      ],
    });
    await this.cmd("DELETE", "/actions");
  }

  /** Double-click at an offset (CSS px) from the element's centre. */
  async doubleClick(css, dx = 0, dy = 0) {
    const el = await this.waitFor(css);
    const click = [{ type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 }];
    await this.cmd("POST", "/actions", {
      actions: [
        {
          type: "pointer",
          id: "mouse",
          parameters: { pointerType: "mouse" },
          actions: [{ type: "pointerMove", origin: { [ELEMENT]: el }, x: Math.round(dx), y: Math.round(dy) }, ...click, ...click],
        },
      ],
    });
    await this.cmd("DELETE", "/actions");
  }

  /** Right-click at viewport coordinates. */
  async contextClickAt(x, y) {
    await this.cmd("POST", "/actions", {
      actions: [{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [{ type: "pointerMove", origin: "viewport", x: Math.round(x), y: Math.round(y) }, { type: "pointerDown", button: 2 }, { type: "pointerUp", button: 2 }] }],
    });
    await this.cmd("DELETE", "/actions");
  }

  /** Left-button drag between two viewport points, in small steps (a real pointer drag). */
  async dragAt(x0, y0, x1, y1, steps = 8) {
    const moves = Array.from({ length: steps }, (_, i) => ({
      type: "pointerMove",
      origin: "viewport",
      duration: 16,
      x: Math.round(x0 + ((x1 - x0) * (i + 1)) / steps),
      y: Math.round(y0 + ((y1 - y0) * (i + 1)) / steps),
    }));
    await this.cmd("POST", "/actions", {
      actions: [
        {
          type: "pointer",
          id: "mouse",
          parameters: { pointerType: "mouse" },
          actions: [{ type: "pointerMove", origin: "viewport", x: Math.round(x0), y: Math.round(y0) }, { type: "pointerDown", button: 0 }, ...moves, { type: "pointerUp", button: 0 }],
        },
      ],
    });
    await this.cmd("DELETE", "/actions");
  }

  /** Double-click at viewport coordinates (for transformed elements whose layout box differs from what is drawn). */
  async doubleClickAt(x, y) {
    const click = [{ type: "pointerDown", button: 0 }, { type: "pointerUp", button: 0 }];
    await this.cmd("POST", "/actions", {
      actions: [{ type: "pointer", id: "mouse", parameters: { pointerType: "mouse" }, actions: [{ type: "pointerMove", origin: "viewport", x: Math.round(x), y: Math.round(y) }, ...click, ...click] }],
    });
    await this.cmd("DELETE", "/actions");
  }

  async screenshot() {
    // Let the page paint what the DOM now holds (WebKitGTK can otherwise return an older frame).
    await this.exec("return new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r(1))))");
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
    await fetch(`${this.base}/session/${this.id}`, { method: "DELETE", signal: AbortSignal.timeout(20_000) }).catch(() => undefined);
  }
}
