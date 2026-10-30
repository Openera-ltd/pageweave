// "All tools" pane (inspired by Acrobat's tools pane): every feature, grouped by what it is for,
// with a filter. Each entry runs the same command as its menu item and shortcut, and is disabled
// with its reason when it cannot run.

import { useMemo, useState } from "react";
import { ArrowRightLeft, X } from "lucide-react";
import { COMMAND_BY_ID, type CommandHandlers, type CommandId } from "../commands";
import { COMMAND_ICONS } from "../commandIcons";

export interface ToolGroup {
  id: string;
  title: string;
  commands: CommandId[];
}

export const TOOL_GROUPS: ToolGroup[] = [
  { id: "convert", title: "Convert and export", commands: ["file.toWord", "file.toPowerPoint", "file.exportTables", "pages.exportText", "pages.exportImages", "pages.extractImages"] },
  { id: "translate", title: "Translate and read aloud", commands: ["file.translate", "read.aloud", "read.saveAudio"] },
  { id: "edit", title: "Edit", commands: ["edit.toggleEditMode", "edit.addText", "file.editProperties", "file.editBookmarks", "file.attachFiles"] },
  { id: "comment", title: "Comment", commands: ["annotate.highlight", "annotate.underline", "annotate.strikeOut", "annotate.note"] },
  { id: "organize", title: "Organize pages", commands: ["pages.organize", "pages.rotate", "pages.delete", "pages.extract", "pages.split", "pages.merge", "pages.crop", "pages.labels", "pages.removeBlank"] },
  { id: "scan", title: "Scan and OCR", commands: ["pages.ocr", "pages.deskew", "pages.orient"] },
  { id: "stamp", title: "Numbers and watermarks", commands: ["pages.stamp", "pages.watermark"] },
  { id: "forms", title: "Forms", commands: ["file.exportFormData", "file.importFormData", "edit.clearForm", "edit.resetForm"] },
  { id: "protect", title: "Protect and redact", commands: ["file.protect", "file.unprotect", "redact.markArea", "redact.markPatterns", "redact.markPage", "redact.apply", "file.sanitize"] },
  { id: "sign", title: "Sign", commands: ["file.sign", "file.signatureMark", "view.showSignatures", "file.trustedCertificates"] },
  { id: "check", title: "Compare and check", commands: ["file.compare", "file.accessibility", "file.reduceSize"] },
  { id: "create", title: "Create and combine", commands: ["file.imagesToPdf", "file.openOffice", "file.batch"] },
];

const label = (id: CommandId) => COMMAND_BY_ID.get(id)!.label.replace(/…$/, "");

/** Whether `text` matches the filter `q` (lower case): a word starting with it, or the whole phrase. */
export function matches(text: string, q: string): boolean {
  if (!q) return true;
  const t = text.toLowerCase();
  return q.includes(" ") ? t.includes(q) : t.split(/[^\p{L}\p{N}]+/u).some((w) => w.startsWith(q));
}

interface Props {
  handlers: CommandHandlers;
  bindings: Record<string, string[]>;
  onClose: () => void;
}

export function ToolsPanel({ handlers, bindings, onClose }: Props) {
  const [filter, setFilter] = useState("");
  const q = filter.trim().toLowerCase();
  const groups = useMemo(
    () =>
      TOOL_GROUPS.map((g) => ({ ...g, commands: g.commands.filter((c) => matches(label(c), q) || matches(g.title, q)) })).filter(
        (g) => g.commands.length > 0,
      ),
    [q],
  );
  return (
    <aside className="tools-pane" aria-label="All tools" data-testid="tools-pane">
      <div className="tools-head">
        <h2>All tools</h2>
        <button type="button" className="btn icon" aria-label="Close all tools" title="Close all tools" data-testid="tools-close" onClick={onClose}>
          <X />
        </button>
      </div>
      <input className="input" aria-label="Find a tool" placeholder="Find a tool" value={filter} data-testid="tools-filter" onChange={(e) => setFilter(e.target.value)} />
      <div className="tools-list">
        {groups.map((g) => (
          <section key={g.id} className="tools-group" aria-labelledby={`tools-${g.id}`}>
            <h3 id={`tools-${g.id}`}>{g.title}</h3>
            <ul>
              {g.commands.map((c) => {
                const avail = handlers[c].available();
                const key = bindings[c]?.[0];
                return (
                  <li key={c}>
                    <button
                      type="button"
                      className="tool-item"
                      data-tool={c}
                      aria-disabled={avail === true ? undefined : true}
                      title={avail === true ? (key ? `${label(c)} (${key})` : label(c)) : `${label(c)} — ${avail}`}
                      onClick={() => {
                        if (handlers[c].available() === true) handlers[c].run();
                      }}
                    >
                      {COMMAND_ICONS[c]}
                      <span>{label(c)}</span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </section>
        ))}
        {groups.length === 0 && <p className="hint">No tool matches “{filter}”.</p>}
      </div>
      <p className="hint tools-foot">
        <ArrowRightLeft aria-hidden="true" /> Every tool is also in the command palette (Ctrl+K).
      </p>
    </aside>
  );
}
