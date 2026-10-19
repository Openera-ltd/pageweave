// Shared interactive primitives. Every actionable control in the app is built from these so that
// accessible names, disabled reasons, keyboard behaviour and focus handling are uniform.

import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { X } from "lucide-react";
import { COMMAND_BY_ID, type CommandHandlers, type CommandId } from "../commands";

export function shortcutLabel(bindings: Record<string, string[]>, id: CommandId): string | undefined {
  return bindings[id]?.[0];
}

interface CommandButtonProps {
  command: CommandId;
  handlers: CommandHandlers;
  bindings: Record<string, string[]>;
  icon?: ReactNode;
  /** Show the label text next to the icon. Icon-only buttons still get an accessible name. */
  showLabel?: boolean;
  label?: string;
  className?: string;
  pressed?: boolean;
}

/**
 * A button bound to a registry command. When unavailable it stays focusable with
 * aria-disabled and a tooltip explaining why, instead of disappearing or silently doing nothing.
 */
export function CommandButton({ command, handlers, bindings, icon, showLabel, label, className, pressed }: CommandButtonProps) {
  const spec = COMMAND_BY_ID.get(command)!;
  const h = handlers[command];
  const avail = h.available();
  const name = label ?? spec.label.replace(/…$/, "");
  const key = shortcutLabel(bindings, command);
  const title = avail === true ? (key ? `${name} (${key})` : name) : `${name} — ${avail}`;
  return (
    <button
      type="button"
      className={`btn ${showLabel ? "" : "icon"} ${className ?? ""}`}
      data-command={command}
      aria-label={showLabel ? undefined : name}
      aria-disabled={avail === true ? undefined : true}
      aria-pressed={pressed}
      aria-keyshortcuts={key}
      title={title}
      onClick={() => {
        if (h.available() === true) h.run();
      }}
    >
      {icon}
      {showLabel && <span>{name}</span>}
    </button>
  );
}

export interface MenuEntry {
  id: string;
  label: string;
  shortcut?: string;
  icon?: ReactNode;
  /** true = enabled, string = disabled reason. */
  available: true | string;
  run: () => void;
}
export type MenuItemOrSeparator = MenuEntry | "separator";

interface MenuProps {
  items: MenuItemOrSeparator[];
  /** Anchor point in viewport coordinates. */
  x: number;
  y: number;
  label: string;
  onClose: () => void;
}

/** Context/dropdown menu: kept inside the window, arrow/Home/End/Enter/Escape, restores focus. */
export function Menu({ items, x, y, label, onClose }: MenuProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y });
  const returnFocus = useRef<Element | null>(document.activeElement);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const margin = 4;
    let left = x;
    let top = y;
    if (left + r.width > window.innerWidth - margin) left = Math.max(margin, window.innerWidth - r.width - margin);
    if (top + r.height > window.innerHeight - margin) top = Math.max(margin, y - r.height);
    setPos({ left, top });
    el.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
  }, [x, y]);

  const close = useCallback(() => {
    onClose();
    const el = returnFocus.current as HTMLElement | null;
    if (el && document.contains(el)) el.focus();
  }, [onClose]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) close();
    };
    const onBlur = () => close();
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("blur", onBlur);
    window.addEventListener("resize", onBlur);
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("resize", onBlur);
    };
  }, [close]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    const all = Array.from(ref.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? []);
    const i = all.indexOf(document.activeElement as HTMLElement);
    const focus = (n: number) => all[(n + all.length) % all.length]?.focus();
    if (e.key === "ArrowDown") focus(i + 1);
    else if (e.key === "ArrowUp") focus(i - 1);
    else if (e.key === "Home") focus(0);
    else if (e.key === "End") focus(all.length - 1);
    else if (e.key === "Escape" || e.key === "Tab") close();
    else return;
    e.preventDefault();
    e.stopPropagation();
  };

  return (
    <div ref={ref} className="menu" role="menu" aria-label={label} style={pos} onKeyDown={onKeyDown}>
      {items.map((it, i) =>
        it === "separator" ? (
          <div key={`sep-${i}`} role="separator" />
        ) : (
          <button
            key={`${it.id}-${i}`}
            type="button"
            role="menuitem"
            data-command={it.id}
            aria-disabled={it.available === true ? undefined : true}
            title={it.available === true ? undefined : it.available}
            onClick={() => {
              if (it.available !== true) return;
              close();
              it.run();
            }}
          >
            {it.icon}
            <span>{it.label}</span>
            {it.available !== true && <span className="sr-only">, unavailable: {it.available}</span>}
            {it.shortcut && <span className="kbd">{it.shortcut}</span>}
          </button>
        ),
      )}
    </div>
  );
}

interface DialogProps {
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  wide?: boolean;
  /** Nearly the whole window (side-by-side views). */
  full?: boolean;
  /** While busy, Escape and the close button are disabled so work is not abandoned by accident. */
  busy?: boolean;
  initialFocus?: string;
}

/** Modal dialog: focus trap, initial focus, Escape to close, focus restored to the opener. */
export function Dialog({ title, onClose, children, footer, wide, full, busy, initialFocus }: DialogProps) {
  const ref = useRef<HTMLDivElement>(null);
  const opener = useRef<Element | null>(document.activeElement);
  const titleId = useRef(`dlg-${Math.random().toString(36).slice(2)}`).current;

  useEffect(() => {
    const el = ref.current;
    const target = (initialFocus && el?.querySelector<HTMLElement>(initialFocus)) || el?.querySelector<HTMLElement>("input, select, textarea, button:not(.dialog-close)");
    target?.focus();
    const openerEl = opener.current as HTMLElement | null;
    return () => {
      if (openerEl && document.contains(openerEl)) openerEl.focus();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      if (!busy) onClose();
      return;
    }
    if (e.key !== "Tab") return;
    const focusables = Array.from(
      ref.current?.querySelectorAll<HTMLElement>('button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])') ?? [],
    ).filter((f) => !f.hasAttribute("disabled"));
    if (focusables.length === 0) return;
    const first = focusables[0]!;
    const last = focusables[focusables.length - 1]!;
    if (e.shiftKey && document.activeElement === first) {
      last.focus();
      e.preventDefault();
    } else if (!e.shiftKey && document.activeElement === last) {
      first.focus();
      e.preventDefault();
    }
  };

  return (
    <div className="backdrop" onMouseDown={(e) => e.target === e.currentTarget && !busy && onClose()}>
      <div ref={ref} className={`dialog ${wide ? "wide" : ""} ${full ? "full" : ""}`} role="dialog" aria-modal="true" aria-labelledby={titleId} onKeyDown={onKeyDown}>
        <div className="dialog-header">
          <h2 id={titleId}>{title}</h2>
          <button type="button" className="btn icon dialog-close" aria-label="Close" title={busy ? "Wait for the task to finish or cancel it" : "Close (Esc)"} disabled={busy} onClick={onClose}>
            <X />
          </button>
        </div>
        <div className="body">{children}</div>
        {footer && <div className="dialog-footer">{footer}</div>}
      </div>
    </div>
  );
}

/** Debounce a value (for live validation without a request per keystroke). */
export function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}
