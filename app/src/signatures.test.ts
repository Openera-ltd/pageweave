import { describe, expect, it } from "vitest";
import { formatTime, signatureSummary, statusLabel } from "./signatures";
import type { SignatureInfo } from "./types";

const sig = (p: Partial<SignatureInfo>): SignatureInfo => ({
  field: "Approval",
  page: 0,
  status: "valid",
  detail: "",
  signer: "Test",
  signer_subject: null,
  issuer: null,
  certificate_valid_from: null,
  certificate_valid_to: null,
  self_signed: false,
  signing_time: null,
  reason: null,
  location: null,
  sub_filter: null,
  digest_algorithm: null,
  signature_algorithm: null,
  covers_whole_file: true,
  changed_after: false,
  has_timestamp: false,
  trust: "trusted",
  trust_detail: "",
  trusted_by: "Test CA",
  chain: ["Test", "Test CA"],
  revocation: "not_checked",
  ...p,
});
const report = (s: SignatureInfo[]) => ({ signatures: s, trust_note: "" });

describe("signature wording", () => {
  it("labels each status and keeps validity apart from trust", () => {
    expect(statusLabel(sig({}))).toEqual({ text: "Valid, trusted", tone: "ok" });
    expect(statusLabel(sig({ trust: "unknown_signer" }))).toEqual({ text: "Valid, signer unknown", tone: "warn" });
    expect(statusLabel(sig({ trust: "expired" })).text).toBe("Valid, certificate expired");
    expect(statusLabel(sig({ trust: "chain_invalid" })).text).toBe("Valid, certificate chain broken");
    expect(statusLabel(sig({ changed_after: true })).tone).toBe("warn");
    expect(statusLabel(sig({ status: "invalid" }))).toEqual({ text: "Invalid", tone: "bad" });
    expect(statusLabel(sig({ status: "unsupported" })).text).toBe("Not checked");
  });

  it("summarises for the status bar, worst first", () => {
    expect(signatureSummary(report([]))).toBeNull();
    expect(signatureSummary(report([sig({})]))).toEqual({ text: "Signed", tone: "ok" });
    expect(signatureSummary(report([sig({ changed_after: true })]))?.text).toBe("Signed, changed since");
    // An earlier signature followed by a later one: the later one decides "changed since".
    expect(signatureSummary(report([sig({ changed_after: true }), sig({})]))).toEqual({ text: "Signed (2)", tone: "ok" });
    expect(signatureSummary(report([sig({}), sig({ status: "invalid" })]))?.tone).toBe("bad");
    expect(signatureSummary(report([sig({ status: "unreadable" })]))?.text).toBe("Signature not checked");
    expect(signatureSummary(report([sig({ trust: "unknown_signer" })]))).toEqual({ text: "Signed, signer not trusted", tone: "warn" });
  });

  it("formats ISO times and keeps other text", () => {
    expect(formatTime(null)).toBeNull();
    expect(formatTime("D:20260101")).toBe("D:20260101");
    expect(formatTime("2026-03-01T12:00:00Z")).not.toContain("T12");
  });
});
