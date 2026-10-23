import { useEffect, useRef, useState } from "react";
import { useApi } from "../api";
import { formatBytes } from "../format";
import { COMMANDS, bindingConflicts, effectiveBindings, type CommandId } from "../commands";
import { eventToShortcut, validateShortcut } from "../shortcuts";
import { toPwError, type Density, type Motion, type PwError, type ResourceReport, type Settings, type Theme } from "../types";
import { Dialog } from "./ui";

export const ACCENTS = ["#2f6fde", "#0f7b8a", "#1c7c3d", "#7a4ddb", "#c2410c", "#b42318", "#4b5563"];

export const DEFAULT_APPEARANCE: Pick<Settings, "theme" | "accent" | "density" | "ui_scale" | "motion"> = {
  theme: "system",
  accent: "#2f6fde",
  density: "compact",
  ui_scale: 1,
  motion: "system",
};

interface Props {
  initial: Settings;
  initialTab?: "appearance" | "shortcuts" | "resources";
  /** Live preview: apply without saving. */
  onPreview: (s: Settings) => void;
  onSave: (s: Settings) => Promise<void>;
  /** Cancel restores exactly the settings that were active when the dialog opened. */
  onCancel: () => void;
}

function Segmented<T extends string>({ label, value, options, onChange }: { label: string; value: T; options: Array<[T, string]>; onChange: (v: T) => void }) {
  const id = `seg-${label.replace(/\s+/g, "-").toLowerCase()}`;
  return (
    <div className="field">
      <span className="label" id={id}>
        {label}
      </span>
      <div className="segmented" role="radiogroup" aria-labelledby={id}>
        {options.map(([v, l]) => (
          <button key={v} type="button" role="radio" aria-checked={value === v} onClick={() => onChange(v)}>
            {l}
          </button>
        ))}
      </div>
    </div>
  );
}

export function SettingsDialog({ initial, initialTab = "appearance", onPreview, onSave, onCancel }: Props) {
  const [s, setS] = useState<Settings>(initial);
  const [tab, setTab] = useState(initialTab);
  const [customAccent, setCustomAccent] = useState(ACCENTS.includes(initial.accent) ? "" : initial.accent);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<PwError | null>(null);

  const update = (patch: Partial<Settings>) => {
    const next = { ...s, ...patch };
    setS(next);
    onPreview(next);
  };

  const conflicts = bindingConflicts(s.shortcuts);
  const hasConflicts = conflicts.size > 0;

  const save = async () => {
    if (hasConflicts) return;
    setSaving(true);
    setError(null);
    try {
      await onSave(s);
    } catch (e) {
      setError(toPwError(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog
      title="Settings"
      onClose={onCancel}
      wide
      busy={saving}
      footer={
        <>
          <button type="button" className="btn outline" onClick={onCancel}>
            Cancel
          </button>
          <button type="button" className="btn primary" aria-disabled={hasConflicts || saving || undefined} title={hasConflicts ? "Resolve shortcut conflicts first" : undefined} onClick={save}>
            {saving ? "Saving…" : "Save"}
          </button>
        </>
      }
    >
      <div className="tabs-inline" role="tablist" aria-label="Settings sections">
        <button type="button" role="tab" id="tab-appearance" aria-controls="panel-appearance" aria-selected={tab === "appearance"} onClick={() => setTab("appearance")}>
          Appearance
        </button>
        <button type="button" role="tab" id="tab-shortcuts" aria-controls="panel-shortcuts" aria-selected={tab === "shortcuts"} onClick={() => setTab("shortcuts")}>
          Keyboard shortcuts
        </button>
        <button type="button" role="tab" id="tab-resources" aria-controls="panel-resources" aria-selected={tab === "resources"} onClick={() => setTab("resources")}>
          Resources
        </button>
      </div>

      {tab === "appearance" && (
        <div role="tabpanel" id="panel-appearance" aria-labelledby="tab-appearance">
          <Segmented<Theme> label="Theme" value={s.theme} options={[["system", "Follow system"], ["light", "Light"], ["dark", "Dark"]]} onChange={(theme) => update({ theme })} />
          <div className="field">
            <span className="label" id="accent-label">
              Accent color
            </span>
            <div className="swatches" role="radiogroup" aria-labelledby="accent-label">
              {ACCENTS.map((a) => (
                <button key={a} type="button" role="radio" className="swatch" aria-checked={s.accent === a} aria-label={`Accent ${a}`} title={a} style={{ background: a }} onClick={() => update({ accent: a })} />
              ))}
              <input
                className="input"
                style={{ width: "9ch" }}
                aria-label="Custom accent color (hex, e.g. #2f6fde)"
                placeholder="#rrggbb"
                value={customAccent}
                aria-invalid={customAccent !== "" && !/^#[0-9a-fA-F]{6}$/.test(customAccent)}
                onChange={(e) => {
                  const v = e.target.value.trim();
                  setCustomAccent(v);
                  if (/^#[0-9a-fA-F]{6}$/.test(v)) update({ accent: v.toLowerCase() });
                }}
              />
            </div>
          </div>
          <Segmented<Density> label="Density" value={s.density} options={[["compact", "Compact"], ["comfortable", "Comfortable"]]} onChange={(density) => update({ density })} />
          <div className="field">
            <label htmlFor="ui-scale">Interface text size: {Math.round(s.ui_scale * 100)}%</label>
            <input id="ui-scale" type="range" min={0.8} max={2} step={0.05} value={s.ui_scale} aria-valuetext={`${Math.round(s.ui_scale * 100)} percent`} onChange={(e) => update({ ui_scale: Number(e.target.value) })} />
          </div>
          <Segmented<Motion> label="Motion" value={s.motion} options={[["system", "Follow system"], ["reduced", "Reduced"], ["full", "Full"]]} onChange={(motion) => update({ motion })} />
          <button type="button" className="btn outline" onClick={() => { setCustomAccent(""); update({ ...DEFAULT_APPEARANCE }); }}>
            Restore default appearance
          </button>
        </div>
      )}

      {tab === "resources" && (
        <div role="tabpanel" id="panel-resources" aria-labelledby="tab-resources">
          <ResourcesPanel />
        </div>
      )}

      {tab === "shortcuts" && (
        <div role="tabpanel" id="panel-shortcuts" aria-labelledby="tab-shortcuts">
          <ShortcutEditor overrides={s.shortcuts} conflicts={conflicts} onChange={(shortcuts) => update({ shortcuts })} />
        </div>
      )}

      {error && (
        <div className="banner error" role="alert">
          {error.message}
        </div>
      )}
    </Dialog>
  );
}

function ShortcutEditor({ overrides, conflicts, onChange }: { overrides: Record<string, string>; conflicts: Map<string, string[]>; onChange: (o: Record<string, string>) => void }) {
  const [filter, setFilter] = useState("");
  const [capturing, setCapturing] = useState<CommandId | null>(null);
  const [captureError, setCaptureError] = useState<string | null>(null);
  const captureRef = useRef<HTMLInputElement>(null);
  const bindings = effectiveBindings(overrides);
  const conflicted = new Set([...conflicts.values()].flat());


  const onCaptureKey = (e: React.KeyboardEvent, id: CommandId) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape" && !e.ctrlKey && !e.altKey && !e.shiftKey) {
      setCapturing(null);
      setCaptureError(null);
      return;
    }
    const sc = eventToShortcut(e);
    if (!sc) return;
    const v = validateShortcut(sc);
    if (!v.ok) {
      setCaptureError(v.reason);
      return;
    }
    onChange({ ...overrides, [id]: v.value });
    setCapturing(null);
    setCaptureError(null);
  };

  const rows = COMMANDS.filter((c) => `${c.label} ${c.category} ${(bindings[c.id] ?? []).join(" ")}`.toLowerCase().includes(filter.toLowerCase()));

  return (
    <>
      <div className="row" style={{ marginBottom: 12 }}>
        <input className="input grow" aria-label="Search shortcuts" placeholder="Search commands or keys" value={filter} onChange={(e) => setFilter(e.target.value)} />
        <button type="button" className="btn outline" onClick={() => onChange({})}>
          Reset all to defaults
        </button>
      </div>
      {conflicts.size > 0 && (
        <div className="banner error" role="alert" style={{ marginBottom: 12, borderRadius: 6 }}>
          {[...conflicts].map(([k, ids]) => `${k} is used by ${ids.map((i) => COMMANDS.find((c) => c.id === i)?.label).join(" and ")}`).join(". ")}. Change one of them.
        </div>
      )}
      <table className="shortcut-table">
        <thead>
          <tr>
            <th scope="col">Command</th>
            <th scope="col">Shortcut</th>
            <th scope="col">
              <span className="sr-only">Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((c) => (
            <tr key={c.id} data-shortcut-row={c.id}>
              <td>{c.label}</td>
              <td>
                {capturing === c.id ? (
                  <>
                    <input
                      ref={captureRef}
                      // Focus on mount (not in an effect) so keys pressed right after "Change" are captured.
                      autoFocus
                      readOnly
                      className="input"
                      style={{ width: "14ch" }}
                      value="Press keys…"
                      aria-label={`Press the new shortcut for ${c.label}, or Escape to cancel`}
                      onKeyDown={(e) => onCaptureKey(e, c.id)}
                      onBlur={() => setCapturing(null)}
                    />
                    {captureError && (
                      <span className="error" role="alert" style={{ marginLeft: 8 }}>
                        {captureError}
                      </span>
                    )}
                  </>
                ) : (bindings[c.id] ?? []).length ? (
                  (bindings[c.id] ?? []).map((k) => (
                    <kbd key={k} style={conflicted.has(c.id) ? { borderColor: "var(--danger)" } : undefined}>
                      {k}
                    </kbd>
                  ))
                ) : (
                  <span className="hint">None</span>
                )}
              </td>
              <td style={{ whiteSpace: "nowrap" }}>
                <button type="button" className="btn" onClick={() => { setCapturing(c.id); setCaptureError(null); }}>
                  Change
                </button>
                <button type="button" className="btn" aria-disabled={overrides[c.id] === undefined || undefined} title={overrides[c.id] === undefined ? "Already the default" : undefined} onClick={() => {
                  if (overrides[c.id] === undefined) return;
                  const { [c.id]: _removed, ...rest } = overrides;
                  onChange(rest);
                }}>
                  Default
                </button>
                <button type="button" className="btn" aria-disabled={(bindings[c.id] ?? []).length === 0 || undefined} title={(bindings[c.id] ?? []).length === 0 ? "No shortcut assigned" : undefined} onClick={() => (bindings[c.id] ?? []).length && onChange({ ...overrides, [c.id]: "" })}>
                  Remove
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </>
  );
}

const STATE_LABEL = { ready: "Ready", missing: "Missing", damaged: "Damaged" } as const;

/** What is installed for each local capability, verified against the pinned checksums. */
function ResourcesPanel() {
  const api = useApi();
  const [report, setReport] = useState<ResourceReport | null>(null);
  const [error, setError] = useState<PwError | null>(null);
  const [checking, setChecking] = useState(false);
  const check = () => {
    setChecking(true);
    setError(null);
    api
      .resourceStatus()
      .then((r) => setReport(r))
      .catch((e) => setError(toPwError(e)))
      .finally(() => setChecking(false));
  };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(check, []);
  return (
    <div className="field">
      <ul className="a11y-list" aria-label="Installed resources" aria-busy={checking} data-testid="resources-list">
        {(report?.capabilities ?? []).map((c) => (
          <li key={c.id} data-resource={c.id} data-state={c.state}>
            <div className="row" style={{ minWidth: 0 }}>
              <span className={`a11y-status ${c.state === "ready" ? "pass" : c.required ? "fail" : "not_applicable"}`}>{!c.required && c.state === "missing" ? "Not installed" : STATE_LABEL[c.state]}</span>
              <strong className="grow">{c.title}</strong>
              {c.bytes > 0 && <span className="hint">{formatBytes(c.bytes)}</span>}
            </div>
            <div className="hint">{c.detail}</div>
          </li>
        ))}
      </ul>
      {!report && !error && <span className="hint">Checking…</span>}
      {error && (
        <p className="error" role="alert">
          {error.message}
        </p>
      )}
      <div className="row">
        <button type="button" className="btn outline" data-testid="resources-check" aria-disabled={checking ? true : undefined} onClick={() => !checking && check()}>
          {checking ? "Checking…" : "Check again"}
        </button>
        <span className="hint">These run on this computer; nothing is downloaded.</span>
      </div>
    </div>
  );
}
