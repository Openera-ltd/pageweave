// Main toolbar: the most used tools as icon buttons in labelled groups, a few dropdowns for the
// longer lists, and an overflow menu. Groups that do not fit the window move into "More tools" in
// priority order, so every command shown here stays reachable at any width.

import { useCallback, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import {
  ChevronDown,
  Command as CommandIcon,
  Ellipsis,
  FileOutput,
  FolderOpen,
  Layers,
  LayoutPanelLeft,
  Menu as MenuIcon,
  Settings as SettingsIcon,
} from "lucide-react";
import { type CommandHandlers, type CommandId } from "../commands";
import { CommandButton } from "./ui";
import { COMMAND_ICONS } from "../commandIcons";

export { COMMAND_ICONS as MENU_ICONS };

/** A toolbar entry: a command button, or a dropdown listing several commands. */
export type BarItem =
  | { kind: "command"; command: CommandId; icon: ReactNode; label?: string; showLabel?: boolean }
  | { kind: "menu"; id: string; label: string; icon: ReactNode; commands: (CommandId | "separator")[] };

export interface BarGroup {
  id: string;
  /** Accessible name of the group. */
  label: string;
  /** Lower numbers stay visible longest when the window narrows. */
  priority: number;
  items: BarItem[];
}

const cmd = (command: CommandId): BarItem => ({ kind: "command", command, icon: COMMAND_ICONS[command] });

export const PAGES_MENU: (CommandId | "separator")[] = [
  "pages.organize",
  "separator",
  "pages.extract",
  "pages.delete",
  "pages.rotate",
  "pages.split",
  "pages.crop",
  "pages.labels",
  "pages.removeBlank",
  "separator",
  "pages.stamp",
  "pages.watermark",
  "separator",
  "pages.ocr",
  "pages.deskew",
  "pages.orient",
  "separator",
  "pages.merge",
];

export const CONVERT_MENU: (CommandId | "separator")[] = [
  "file.toWord",
  "file.toPowerPoint",
  "file.exportTables",
  "separator",
  "pages.exportText",
  "pages.exportImages",
  "pages.extractImages",
  "separator",
  "file.translate",
  "read.saveAudio",
];

/** The groups between the fixed start (menu, open) and end (palette, all tools, settings). */
export function barGroups(state: { reading: boolean; editMode: boolean }): BarGroup[] {
  return [
    { id: "file", label: "File", priority: 1, items: [cmd("file.saveCopy"), cmd("file.print")] },
    { id: "history", label: "Undo and redo", priority: 2, items: [cmd("edit.undo"), cmd("edit.redo")] },
    { id: "view", label: "View", priority: 1, items: [cmd("view.toggleThumbnails"), cmd("search.open")] },
    {
      id: "comment",
      label: "Comment",
      priority: 5,
      items: [cmd("annotate.highlight"), cmd("annotate.underline"), cmd("annotate.strikeOut"), cmd("annotate.note")],
    },
    {
      id: "edit",
      label: "Edit",
      priority: 3,
      items: [cmd("edit.toggleEditMode"), ...(state.editMode ? [cmd("edit.addText")] : [])],
    },
    {
      id: "protect",
      label: "Sign and protect",
      priority: 6,
      items: [cmd("file.signatureMark"), cmd("redact.markArea"), cmd("file.protect")],
    },
    {
      id: "pages",
      label: "Pages",
      priority: 4,
      items: [cmd("pages.organize"), cmd("pages.rotate"), { kind: "menu", id: "pages", label: "Pages", icon: <Layers />, commands: PAGES_MENU }],
    },
    {
      id: "convert",
      label: "Convert and read",
      priority: 4,
      items: [
        { kind: "menu", id: "convert", label: "Convert", icon: <FileOutput />, commands: CONVERT_MENU },
        cmd("file.translate"),
        cmd("pages.ocr"),
        cmd(state.reading ? "read.stop" : "read.aloud"),
      ],
    },
  ];
}

/** Which groups fit in `available` px, keeping the lowest priority numbers; ties keep the leftmost. */
export function fitGroups(groups: { id: string; priority: number }[], widths: Record<string, number>, available: number, moreWidth: number): Set<string> {
  const total = groups.reduce((n, g) => n + (widths[g.id] ?? 0), 0);
  if (total <= available) return new Set();
  const order = groups.map((g, i) => ({ ...g, i })).sort((a, b) => a.priority - b.priority || a.i - b.i);
  const hidden = new Set<string>();
  let used = moreWidth;
  for (const g of order) {
    const w = widths[g.id] ?? 0;
    if (hidden.size === 0 && used + w <= available) used += w;
    else hidden.add(g.id);
  }
  return hidden;
}

/** Commands of the hidden groups, flattened for the overflow menu (separators between groups). */
export function overflowCommands(groups: BarGroup[], hidden: Set<string>): (CommandId | "separator")[] {
  const out: (CommandId | "separator")[] = [];
  for (const g of groups) {
    if (!hidden.has(g.id)) continue;
    if (out.length) out.push("separator");
    // A command can be both a button and a dropdown entry (Translate): list it once.
    for (const c of g.items.flatMap((it) => (it.kind === "command" ? [it.command] : it.commands))) if (c !== "separator" && !out.includes(c)) out.push(c);
  }
  return out;
}

interface Props {
  handlers: CommandHandlers;
  bindings: Record<string, string[]>;
  groups: BarGroup[];
  /** Label of the open dropdown/menu, to mark its button expanded. */
  openMenu: string | null;
  onAppMenu: (anchor: DOMRect) => void;
  onMenu: (label: string, anchor: DOMRect, commands: (CommandId | "separator")[]) => void;
  pressed: Partial<Record<CommandId, boolean>>;
  /** Changes when density or text size change (toolbar widths are measured again). */
  scaleKey: string;
  noDocReason: string;
  hasDoc: boolean;
}

export function TopBar({ handlers, bindings, groups, openMenu, onAppMenu, onMenu, pressed, scaleKey, noDocReason, hasDoc }: Props) {
  const area = useRef<HTMLDivElement>(null);
  const widths = useRef<Record<string, number>>({});
  const [hidden, setHidden] = useState<Set<string>>(() => new Set());
  const groupKey = groups.map((g) => `${g.id}:${g.items.length}`).join(",");

  // Measure the groups on screen (hidden ones keep their last width), then decide which fit.
  const fit = useCallback(() => {
    const el = area.current;
    if (!el) return;
    for (const g of el.querySelectorAll<HTMLElement>("[data-group]")) widths.current[g.dataset.group!] = g.offsetWidth;
    // The overflow button is square, as tall as any toolbar button.
    const moreWidth = (el.parentElement?.querySelector<HTMLElement>(".btn")?.offsetHeight ?? 28) + 8;
    const next = fitGroups(groups, widths.current, el.clientWidth, moreWidth);
    setHidden((h) => (h.size === next.size && [...h].every((id) => next.has(id)) ? h : next));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [groupKey]);

  // Density, text size or the set of buttons changed: show everything once so every group is
  // measured afresh (a hidden group's cached width would be stale).
  useLayoutEffect(() => {
    widths.current = {};
    setHidden(new Set());
  }, [scaleKey, groupKey]);

  // Only when what is shown changes, not on every render of the app (reading sizes forces layout).
  useLayoutEffect(() => fit(), [fit, hidden]);

  // Window resizes, and groups changing size (fonts loading, scale applied after this render).
  useLayoutEffect(() => {
    const el = area.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => fit());
    ro.observe(el);
    for (const g of el.querySelectorAll<HTMLElement>("[data-group]")) ro.observe(g);
    return () => ro.disconnect();
  }, [fit, hidden]);

  const more = overflowCommands(groups, hidden);
  const menuTitle = (label: string) => (hasDoc ? label : `${label} — ${noDocReason}`);

  return (
    <div className="toolbar" role="toolbar" aria-label="Main toolbar">
      <div className="group">
        <button
          type="button"
          className="btn icon"
          aria-label="Menu"
          aria-haspopup="menu"
          aria-expanded={openMenu === "Menu"}
          title="Menu: files, preferences and help"
          data-testid="app-menu"
          onClick={(e) => onAppMenu(e.currentTarget.getBoundingClientRect())}
        >
          <MenuIcon />
        </button>
        <CommandButton command="file.open" handlers={handlers} bindings={bindings} icon={<FolderOpen />} showLabel label="Open" />
      </div>
      <div className="toolbar-groups" ref={area}>
        {groups.map((g) =>
          hidden.has(g.id) ? null : (
            <div key={g.id} className="group" role="group" aria-label={g.label} data-group={g.id}>
              {g.items.map((it) =>
                it.kind === "command" ? (
                  <CommandButton key={it.command} command={it.command} handlers={handlers} bindings={bindings} icon={it.icon} showLabel={it.showLabel} label={it.label} pressed={pressed[it.command]} />
                ) : (
                  <button
                    key={it.id}
                    type="button"
                    className="btn dropdown"
                    aria-haspopup="menu"
                    aria-expanded={openMenu === it.label}
                    aria-disabled={hasDoc ? undefined : true}
                    title={menuTitle(it.label)}
                    data-testid={`menu-${it.id}`}
                    onClick={(e) => hasDoc && onMenu(it.label, e.currentTarget.getBoundingClientRect(), it.commands)}
                  >
                    {it.icon}
                    <span>{it.label}</span>
                    <ChevronDown className="chev" aria-hidden="true" />
                  </button>
                ),
              )}
            </div>
          ),
        )}
        {more.length > 0 && (
          <button
            type="button"
            className="btn icon"
            aria-label="More tools"
            aria-haspopup="menu"
            aria-expanded={openMenu === "More tools"}
            title="More tools"
            data-testid="toolbar-more"
            onClick={(e) => onMenu("More tools", e.currentTarget.getBoundingClientRect(), more)}
          >
            <Ellipsis />
          </button>
        )}
      </div>
      <div className="group end">
        <CommandButton command="app.commandPalette" handlers={handlers} bindings={bindings} icon={<CommandIcon />} />
        <CommandButton command="view.toggleTools" handlers={handlers} bindings={bindings} icon={<LayoutPanelLeft />} showLabel label="All tools" pressed={pressed["view.toggleTools"]} />
        <CommandButton command="app.settings" handlers={handlers} bindings={bindings} icon={<SettingsIcon />} />
      </div>
    </div>
  );
}
