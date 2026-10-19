import { useState } from "react";
import { Dialog } from "./ui";

const MAX_NOTE_CHARS = 10_000; // mirrors pw-core MAX_NOTE_CHARS

/** Write the text of a sticky note. Ctrl+Enter adds it. */
export function NoteDialog({ page, onAdd, onCancel }: { page: number; onAdd: (text: string) => void; onCancel: () => void }) {
  const [text, setText] = useState("");
  const tooLong = text.length > MAX_NOTE_CHARS;
  const blocker = !text.trim() ? "Write the note first." : tooLong ? `Notes can be up to ${MAX_NOTE_CHARS.toLocaleString()} characters.` : null;
  const add = () => {
    if (!blocker) onAdd(text);
  };
  return (
    <Dialog
      title="Add note"
      onClose={onCancel}
      footer={
        <>
          <button type="button" className="btn outline" onClick={onCancel}>
            Cancel
          </button>
          <button type="button" className="btn primary" data-testid="note-add" aria-disabled={blocker ? true : undefined} title={blocker ?? "Add note (Ctrl+Enter)"} onClick={add}>
            Add note
          </button>
        </>
      }
    >
      <div className="field">
        <label htmlFor="note-text">Note on page {page + 1}</label>
        <textarea
          id="note-text"
          className="input"
          style={{ height: 120, paddingTop: 6, resize: "vertical" }}
          value={text}
          aria-invalid={tooLong}
          aria-describedby="note-msg"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
              e.preventDefault();
              add();
            }
          }}
        />
        <span id="note-msg" className={tooLong ? "error" : "hint"}>
          {tooLong ? blocker : "Saved into the PDF when you save an annotated copy."}
        </span>
      </div>
    </Dialog>
  );
}

/**
 * Asked before closing a document (or the app) that has unsaved changes (annotations, form fields). The safe choice,
 * Save, has initial focus; Discard is never the default.
 */
export function UnsavedDialog({
  names,
  count,
  marks = 0,
  canSave,
  onSave,
  onDiscard,
  onCancel,
}: {
  /** Documents with unsaved changes. */
  names: string[];
  count: number;
  /** Redaction marks among the changes: never saved into a copy, only applied. */
  marks?: number;
  /** Saving from here is offered only for a single document. */
  canSave: boolean;
  onSave: () => void;
  onDiscard: () => void;
  onCancel: () => void;
}) {
  const one = names.length === 1;
  return (
    <Dialog
      title="Unsaved changes"
      onClose={onCancel}
      initialFocus={canSave ? '[data-testid="unsaved-save"]' : '[data-testid="unsaved-cancel"]'}
      footer={
        <>
          <button type="button" className="btn outline" data-testid="unsaved-cancel" onClick={onCancel}>
            Cancel
          </button>
          <button type="button" className="btn outline" data-testid="unsaved-discard" onClick={onDiscard}>
            Discard
          </button>
          {canSave && (
            <button type="button" className="btn primary" data-testid="unsaved-save" onClick={onSave}>
              Save copy…
            </button>
          )}
        </>
      }
    >
      <p style={{ marginTop: 0 }}>
        {one ? (
          <>
            <strong>{names[0]}</strong> has {count} unsaved change{count === 1 ? "" : "s"}.
          </>
        ) : (
          <>
            {names.length} documents have {count} unsaved changes: {names.join(", ")}.
          </>
        )}
      </p>
      <p className="hint" style={{ marginBottom: 0 }}>
        {canSave ? "Save them to a copy of the PDF, or discard them." : "Cancel to save each document’s changes first, or discard them all."}
      </p>
      {marks > 0 && (
        <p className="hint" data-testid="unsaved-marks" style={{ marginBottom: 0 }}>
          {marks} redaction mark{marks === 1 ? " is" : "s are"} not saved in a copy. Cancel and use Apply redactions to remove that content.
        </p>
      )}
    </Dialog>
  );
}
