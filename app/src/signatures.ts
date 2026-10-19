// Signature status wording shared by the sidebar panel and the status bar. Pure (unit tested).
// "Valid" (unchanged, matches the certificate) and "trusted" (chains to a certificate the user
// trusts) are separate: an unknown signer is always said to be unknown.

import type { SignatureInfo, SignatureReport } from "./types";

export type Tone = "ok" | "warn" | "bad";

export function statusLabel(s: SignatureInfo): { text: string; tone: Tone } {
  switch (s.status) {
    case "valid": {
      if (s.trust !== "trusted") {
        const why = s.trust === "expired" ? "certificate expired" : s.trust === "chain_invalid" ? "certificate chain broken" : "signer unknown";
        return { text: `Valid, ${why}`, tone: "warn" };
      }
      return s.changed_after ? { text: "Trusted, document changed since", tone: "warn" } : { text: "Valid, trusted", tone: "ok" };
    }
    case "invalid":
      return { text: "Invalid", tone: "bad" };
    case "unsupported":
      return { text: "Not checked", tone: "warn" };
    default:
      return { text: "Unreadable", tone: "warn" };
  }
}

/** One-line summary for the status bar, or null when the document has no signatures. */
export function signatureSummary(r: SignatureReport | null): { text: string; tone: Tone } | null {
  if (!r || r.signatures.length === 0) return null;
  const n = r.signatures.length;
  if (r.signatures.some((s) => s.status === "invalid")) return { text: n === 1 ? "Signature invalid" : "A signature is invalid", tone: "bad" };
  if (r.signatures.some((s) => s.status !== "valid")) return { text: n === 1 ? "Signature not checked" : "Some signatures not checked", tone: "warn" };
  // Every signature but the last one naturally sees later changes (the later signatures).
  if (r.signatures[n - 1]!.changed_after) return { text: "Signed, changed since", tone: "warn" };
  if (r.signatures.some((s) => s.trust !== "trusted")) return { text: n === 1 ? "Signed, signer not trusted" : "Signed, not all signers trusted", tone: "warn" };
  return { text: n === 1 ? "Signed" : `Signed (${n})`, tone: "ok" };
}

/** "2026-03-01T12:00:00Z" → local date only (certificate validity). */
export function formatDate(t: string | null): string | null {
  if (!t) return null;
  const d = new Date(t);
  return Number.isNaN(d.getTime()) ? t : d.toLocaleDateString();
}

/** "2026-03-01T12:00:00Z" → local date and time; other forms (PDF dates) are shown as they are. */
export function formatTime(t: string | null): string | null {
  if (!t) return null;
  const d = new Date(t);
  return Number.isNaN(d.getTime()) ? t : d.toLocaleString();
}
