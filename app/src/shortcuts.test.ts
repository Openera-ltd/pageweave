import { describe, expect, it } from "vitest";
import { COMMANDS, bindingConflicts, commandForShortcut, effectiveBindings } from "./commands";
import { eventToShortcut, findConflicts, normalizeShortcut, validateShortcut } from "./shortcuts";

const ev = (key: string, mods: Partial<{ ctrl: boolean; alt: boolean; shift: boolean; meta: boolean }> = {}) => ({
  key,
  ctrlKey: !!mods.ctrl,
  altKey: !!mods.alt,
  shiftKey: !!mods.shift,
  metaKey: !!mods.meta,
});

describe("normalizeShortcut", () => {
  it("canonicalizes order, case and aliases", () => {
    expect(normalizeShortcut("shift + ctrl + o")).toBe("Ctrl+Shift+O");
    expect(normalizeShortcut("Control+Esc")).toBe("Ctrl+Escape");
    expect(normalizeShortcut("f3")).toBe("F3");
    expect(normalizeShortcut("Ctrl++")).toBe("Ctrl+Plus");
    expect(normalizeShortcut("Ctrl+pagedown")).toBe("Ctrl+PageDown");
  });
  it("rejects invalid input", () => {
    for (const bad of ["", "Ctrl", "Ctrl+Shift", "Ctrl+A+B", "Ctrl+Foo", "F25"]) {
      expect(normalizeShortcut(bad), bad).toBeNull();
    }
  });
});

describe("eventToShortcut", () => {
  it("maps keyboard events", () => {
    expect(eventToShortcut(ev("o", { ctrl: true }))).toBe("Ctrl+O");
    expect(eventToShortcut(ev("O", { ctrl: true, shift: true }))).toBe("Ctrl+Shift+O");
    expect(eventToShortcut(ev("F3", { shift: true }))).toBe("Shift+F3");
    expect(eventToShortcut(ev("+", { ctrl: true, shift: true }))).toBe("Ctrl+Plus");
    expect(eventToShortcut(ev("=", { ctrl: true }))).toBe("Ctrl+=");
    expect(eventToShortcut(ev("Control", { ctrl: true }))).toBeNull();
    expect(eventToShortcut(ev("Dead"))).toBeNull();
  });
});

describe("validateShortcut", () => {
  it("blocks reserved and typing-hostile bindings", () => {
    expect(validateShortcut("Alt+F4")).toEqual({ ok: false, reason: "Alt+F4 is reserved by Windows." });
    expect(validateShortcut("A").ok).toBe(false);
    expect(validateShortcut("F7")).toEqual({ ok: true, value: "F7" });
    expect(validateShortcut("ctrl+alt+x")).toEqual({ ok: true, value: "Ctrl+Alt+X" });
  });
});

describe("command registry", () => {
  it("has unique IDs and labels", () => {
    expect(new Set(COMMANDS.map((c) => c.id)).size).toBe(COMMANDS.length);
    expect(new Set(COMMANDS.map((c) => c.label)).size).toBe(COMMANDS.length);
  });

  it("default shortcuts are valid, unreserved and conflict-free", () => {
    for (const c of COMMANDS) {
      for (const s of c.shortcuts) {
        expect(normalizeShortcut(s), `${c.id}: ${s}`).not.toBeNull();
        expect(validateShortcut(s).ok, `${c.id}: ${s}`).toBe(true);
      }
    }
    expect(bindingConflicts({})).toEqual(new Map());
  });

  it("overrides replace defaults and conflicts are detected", () => {
    const b = effectiveBindings({ "file.open": "Ctrl+Shift+O", "app.settings": "" });
    expect(b["file.open"]).toEqual(["Ctrl+Shift+O"]);
    expect(b["app.settings"]).toEqual([]);
    expect(commandForShortcut("Ctrl+Shift+O", b)).toBe("file.open");
    expect(commandForShortcut("Ctrl+O", b)).toBeUndefined();
    const conflicts = bindingConflicts({ "pages.split": "Ctrl+O" });
    expect(conflicts.get("Ctrl+O")?.sort()).toEqual(["file.open", "pages.split"]);
    expect(findConflicts({ a: ["Ctrl+X"], b: ["ctrl+x"] }).size).toBe(1);
  });
});
