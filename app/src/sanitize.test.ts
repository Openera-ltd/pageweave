import { describe, expect, it } from "vitest";
import { DEFAULT_SANITIZE, findingSummary } from "./components/SanitizeDialog";
import type { SanitizeFindings } from "./types";

const none: SanitizeFindings = {
  metadata: [],
  scripts: 0,
  attachments: 0,
  attachment_names: [],
  layers: 0,
  hidden_layers: [],
  comments: 0,
  form_fields: 0,
  web_links: 0,
  earlier_versions: false,
};

describe("sanitize dialog", () => {
  it("defaults match the engine: fields and web links are kept", () => {
    expect(DEFAULT_SANITIZE).toEqual({ metadata: true, scripts: true, attachments: true, hidden_layers: true, comments: true, form_fields: false, web_links: false });
  });

  it("says when nothing was found", () => {
    for (const k of Object.keys(DEFAULT_SANITIZE) as Array<keyof typeof DEFAULT_SANITIZE>) {
      expect(findingSummary(k, none)).toBe("None found");
    }
  });

  it("summarises what was found", () => {
    const f: SanitizeFindings = { ...none, metadata: ["Title", "XMP metadata"], scripts: 7, attachments: 2, attachment_names: ["a.txt", "b.txt"], layers: 2, hidden_layers: ["Draft"], comments: 1 };
    expect(findingSummary("metadata", f)).toBe("Title, XMP metadata");
    expect(findingSummary("scripts", f)).toBe("7 found");
    expect(findingSummary("attachments", f)).toBe("2 found: a.txt, b.txt");
    expect(findingSummary("hidden_layers", f)).toBe("Draft");
    expect(findingSummary("hidden_layers", { ...none, layers: 1 })).toBe("None hidden (1 layer)");
    expect(findingSummary("attachments", { ...none, attachments: 1 })).toBe("1 found");
    expect(findingSummary("comments", f)).toBe("1 found");
  });
});
