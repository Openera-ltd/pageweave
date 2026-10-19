import { useEffect, useRef, useState } from "react";
import { Eraser, FolderOpen } from "lucide-react";
import { useApi } from "../api";
import { siblingOutput } from "../paths";
import { SIGN_POSITIONS, signatureRect, type SignPosition } from "../signPlacement";
import { toPwError, type CollisionPolicy, type JobSnapshot, type OpenedDoc, type PwError } from "../types";
import { PlacementPreview } from "./PlacementPreview";
import { Dialog } from "./ui";

interface Props {
  doc: OpenedDoc;
  currentPage: number;
  jobs: Record<string, JobSnapshot>;
  onClose: () => void;
  onFinished: (job: JobSnapshot) => void;
}

type Kind = "typed" | "drawn" | "image";
type Stroke = Array<[number, number]>;

export const MARK_SIZES = {
  signature: { w: 200, h: 60, label: "Signature" },
  initials: { w: 90, h: 45, label: "Initials" },
} as const;

const STYLES = [
  { id: "formal", label: "Formal", css: '"Times New Roman", Times, serif' },
  { id: "plain", label: "Plain", css: "Helvetica, Arial, sans-serif" },
  { id: "typewriter", label: "Typewriter", css: '"Courier New", Courier, monospace' },
] as const;

/** Why typed text cannot be used, or null. Mirrors the engine's checks (pw-core). */
export function typedBlocker(text: string): string | null {
  const t = text.trim();
  if (!t) return "Type your name or initials.";
  if ([...t].length > 100) return "Type at most 100 characters.";
  if ([...t].some((c) => c.codePointAt(0)! > 255 || /\p{Cc}/u.test(c))) return "Typed signatures can use Latin letters only for now. Draw it or use a picture instead.";
  return null;
}

/** Drawing pad: strokes kept as 0..1 coordinates across the pad (y downwards). */
function SignaturePad({ aspect, strokes, onChange }: { aspect: number; strokes: Stroke[]; onChange: (s: Stroke[]) => void }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const drawing = useRef<Stroke | null>(null);
  const width = 420;
  const height = Math.round(width / aspect);

  useEffect(() => {
    const c = ref.current;
    if (!c) return;
    const dpr = window.devicePixelRatio || 1;
    c.width = width * dpr;
    c.height = height * dpr;
    const g = c.getContext("2d");
    if (!g) return;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, width, height);
    g.lineCap = "round";
    g.lineJoin = "round";
    g.lineWidth = 2;
    g.strokeStyle = "#1a2b5c";
    for (const s of strokes) {
      g.beginPath();
      s.forEach(([x, y], i) => (i ? g.lineTo(x * width, y * height) : g.moveTo(x * width, y * height)));
      if (s.length === 1) g.lineTo(s[0]![0] * width + 0.1, s[0]![1] * height);
      g.stroke();
    }
  }, [strokes, width, height]);

  const point = (e: React.PointerEvent): [number, number] => {
    const r = e.currentTarget.getBoundingClientRect();
    const clamp = (v: number) => Math.min(1, Math.max(0, v));
    return [clamp((e.clientX - r.left) / r.width), clamp((e.clientY - r.top) / r.height)];
  };

  return (
    <canvas
      ref={ref}
      className="signature-pad"
      data-testid="signature-pad"
      style={{ width, height }}
      role="img"
      aria-label={strokes.length ? `Signature drawing, ${strokes.length} stroke${strokes.length === 1 ? "" : "s"}` : "Empty signature drawing area"}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        drawing.current = [point(e)];
        onChange([...strokes, drawing.current]);
      }}
      onPointerMove={(e) => {
        if (!drawing.current) return;
        const p = point(e);
        const last = drawing.current[drawing.current.length - 1]!;
        if (Math.hypot(p[0] - last[0], p[1] - last[1]) < 0.003) return;
        drawing.current = [...drawing.current, p];
        onChange([...strokes.slice(0, -1), drawing.current]);
      }}
      onPointerUp={() => (drawing.current = null)}
      onPointerCancel={() => (drawing.current = null)}
    />
  );
}

/** Place a typed, drawn or pictured signature (or initials) on a page of a copy. Not a digital signature. */
export function SignatureMarkDialog({ doc, currentPage, jobs, onClose, onFinished }: Props) {
  const api = useApi();
  const [kind, setKind] = useState<Kind>("typed");
  const [size, setSize] = useState<keyof typeof MARK_SIZES>("signature");
  const [text, setText] = useState("");
  const [style, setStyle] = useState<(typeof STYLES)[number]["id"]>("formal");
  const [strokes, setStrokes] = useState<Stroke[]>([]);
  const [picture, setPicture] = useState("");
  const [page, setPage] = useState(currentPage + 1);
  const [position, setPosition] = useState<SignPosition>("bottom-right");
  const [output, setOutput] = useState(siblingOutput(doc.path, "signed"));
  const [jobId, setJobId] = useState<string | null>(null);
  const [error, setError] = useState<PwError | null>(null);
  const [conflict, setConflict] = useState(false);
  const job = jobId ? jobs[jobId] : undefined;
  const busy = !!job && !["succeeded", "failed", "cancelled"].includes(job.state);

  useEffect(() => {
    if (!job) return;
    if (job.state === "succeeded") {
      onFinished(job);
      onClose();
    } else if (job.state === "failed" && job.error) {
      setJobId(null);
      setError(job.error);
      setConflict(job.error.category === "output_conflict");
    } else if (job.state === "cancelled") {
      setJobId(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [job?.state]);

  const pageInfo = doc.info.pages[page - 1];
  const pageOk = Number.isInteger(page) && !!pageInfo;
  const box = MARK_SIZES[size];
  const rect = pageInfo ? signatureRect(pageInfo, position, box) : null;
  const blocker =
    (kind === "typed" ? typedBlocker(text) : null) ??
    (kind === "drawn" && !strokes.length ? "Draw your signature first." : null) ??
    (kind === "image" && !picture.trim() ? "Choose a picture of your signature (PNG or JPEG)." : null) ??
    (!pageOk ? `Choose a page from 1 to ${doc.info.page_count}.` : null) ??
    (!output.trim() ? "Choose where to save the copy." : null);

  const start = async (collision: CollisionPolicy) => {
    if (blocker || busy || !rect) return;
    setError(null);
    setConflict(false);
    const mark =
      kind === "typed"
        ? { kind, text: text.trim(), style, color: "#1a2b5c" }
        : kind === "drawn"
          ? { kind, strokes, color: "#1a2b5c", width: size === "initials" ? 1.4 : 1.6 }
          : { kind, path: picture };
    try {
      setJobId(
        await api.startOperation(
          {
            schema_version: 1,
            operation: { op: "add_signature_mark", input: doc.path, page: page - 1, rect: { left: rect[0], bottom: rect[1], right: rect[2], top: rect[3] }, mark, output },
            collision,
          },
          doc.doc_id,
        ),
      );
    } catch (e) {
      setError(toPwError(e));
    }
  };

  const footer = (
    <>
      <button type="button" className="btn outline" onClick={onClose} disabled={busy}>
        Cancel
      </button>
      <button type="button" className="btn primary" data-testid="mark-start" aria-disabled={blocker || busy ? true : undefined} title={blocker ?? undefined} onClick={() => start("fail")}>
        {busy ? "Placing…" : "Place"}
      </button>
    </>
  );

  const tab = (k: Kind, label: string) => (
    <button
      key={k}
      type="button"
      role="tab"
      id={`mark-tab-${k}`}
      aria-selected={kind === k}
      aria-controls="mark-panel"
      tabIndex={kind === k ? 0 : -1}
      className={`seg${kind === k ? " active" : ""}`}
      onClick={() => setKind(k)}
      onKeyDown={(e) => {
        const order: Kind[] = ["typed", "drawn", "image"];
        const i = order.indexOf(kind);
        if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
          const next = order[(i + (e.key === "ArrowRight" ? 1 : order.length - 1)) % order.length]!;
          setKind(next);
          document.getElementById(`mark-tab-${next}`)?.focus();
        }
      }}
    >
      {label}
    </button>
  );

  return (
    <Dialog title="Add signature or initials" onClose={onClose} footer={footer} busy={busy} wide initialFocus="#mark-tab-typed">
      <p className="hint" style={{ marginTop: 0 }}>
        Places a picture of your signature on a copy of <strong>{doc.name}</strong>. This is not a digital signature: use Sign with a certificate for one that can be checked.
      </p>
      <div className="row" style={{ gap: "var(--space-3)" }}>
        <div className="segmented" role="tablist" aria-label="How to sign">
          {tab("typed", "Type")}
          {tab("drawn", "Draw")}
          {tab("image", "Picture")}
        </div>
        <div className="field" style={{ marginLeft: "auto" }}>
          <select aria-label="Size" className="select" value={size} onChange={(e) => setSize(e.target.value as keyof typeof MARK_SIZES)}>
            {Object.entries(MARK_SIZES).map(([k, v]) => (
              <option key={k} value={k}>
                {v.label}
              </option>
            ))}
          </select>
        </div>
      </div>
      <div id="mark-panel" role="tabpanel" aria-labelledby={`mark-tab-${kind}`} className="field">
        {kind === "typed" && (
          <>
            <div className="row" style={{ gap: "var(--space-3)" }}>
              <input id="mark-text" className="input grow" aria-label="Name or initials" placeholder="Your name or initials" value={text} maxLength={100} onChange={(e) => setText(e.target.value)} />
              <select aria-label="Lettering" className="select" value={style} onChange={(e) => setStyle(e.target.value as typeof style)}>
                {STYLES.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.label}
                  </option>
                ))}
              </select>
            </div>
            <div className="typed-preview" aria-hidden="true" style={{ fontFamily: STYLES.find((s) => s.id === style)!.css }}>
              {text.trim() || " "}
            </div>
            {text && typedBlocker(text) && <span className="error">{typedBlocker(text)}</span>}
          </>
        )}
        {kind === "drawn" && (
          <>
            <SignaturePad aspect={box.w / box.h} strokes={strokes} onChange={setStrokes} />
            <div className="row" style={{ justifyContent: "space-between" }}>
              <span className="hint">Draw with a mouse, pen or finger. To sign with the keyboard, use Type.</span>
              <button type="button" className="btn outline" data-testid="pad-clear" aria-disabled={strokes.length ? undefined : true} title={strokes.length ? undefined : "Nothing drawn yet."} onClick={() => strokes.length && setStrokes([])}>
                <Eraser /> Clear
              </button>
            </div>
          </>
        )}
        {kind === "image" && (
          <div className="row">
            <input id="mark-picture" className="input grow" aria-label="Picture file" placeholder="PNG or JPEG; a transparent PNG looks best" value={picture} spellCheck={false} onChange={(e) => setPicture(e.target.value)} />
            <button
              type="button"
              className="btn outline"
              onClick={async () => {
                const p = await api.pickImageFiles();
                if (p[0]) setPicture(p[0]);
              }}
            >
              <FolderOpen /> Browse…
            </button>
          </div>
        )}
      </div>
      <div className="row" style={{ gap: "var(--space-4)", alignItems: "flex-start" }}>
        <div className="grow">
          <div className="row" style={{ gap: "var(--space-3)" }}>
            <div className="field">
              <label htmlFor="mark-page">Page</label>
              <input id="mark-page" className="input" type="number" min={1} max={doc.info.page_count} value={Number.isNaN(page) ? "" : page} aria-invalid={!pageOk} style={{ width: 80 }} onChange={(e) => setPage(e.target.valueAsNumber)} />
            </div>
            <div className="field grow">
              <label htmlFor="mark-position">Position</label>
              <select id="mark-position" className="select" value={position} onChange={(e) => setPosition(e.target.value as SignPosition)}>
                {SIGN_POSITIONS.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.label}
                  </option>
                ))}
              </select>
            </div>
          </div>
          <div className="field">
            <label htmlFor="mark-output">Save as</label>
            <div className="row">
              <input id="mark-output" className="input grow" value={output} spellCheck={false} onChange={(e) => setOutput(e.target.value)} />
              <button
                type="button"
                className="btn outline"
                onClick={async () => {
                  const p = await api.pickSavePath(output);
                  if (p) setOutput(p.toLowerCase().endsWith(".pdf") ? p : `${p}.pdf`);
                }}
              >
                <FolderOpen /> Browse…
              </button>
            </div>
          </div>
        </div>
        {pageInfo && rect && <PlacementPreview docId={doc.doc_id} page={pageInfo} rect={rect} />}
      </div>
      {error && (
        <div className="banner error" role="alert" style={{ borderRadius: 6 }}>
          <div className="grow">
            <div>{error.message}</div>
            {!conflict && error.action && <div className="hint">{error.action}</div>}
          </div>
          {conflict && (
            <div className="row">
              <button type="button" className="btn outline" onClick={() => start("rename")}>
                Keep both
              </button>
              <button type="button" className="btn danger" onClick={() => start("replace")}>
                Replace
              </button>
            </div>
          )}
        </div>
      )}
    </Dialog>
  );
}
