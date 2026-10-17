import { describe, expect, it } from "vitest";
import { MAX_PROPERTY_CHARS, propertiesBlocker, propertyChanges } from "./components/EditPropertiesDialog";

const orig = { title: "Report", author: "Ann", subject: "", keywords: "" };

describe("edit document properties", () => {
  it("needs a change before saving", () => {
    expect(propertiesBlocker(orig, orig)).toBe("Change a property first.");
    expect(propertiesBlocker({ ...orig, title: " Report " }, orig)).toBe("Change a property first.");
    expect(propertiesBlocker({ ...orig, title: "New" }, orig)).toBeNull();
  });

  it("mirrors the engine's limits", () => {
    expect(propertiesBlocker({ ...orig, title: "x".repeat(MAX_PROPERTY_CHARS + 1) }, orig)).toMatch(/at most/);
    expect(propertiesBlocker({ ...orig, author: "a\tb" }, orig)).toMatch(/control characters/);
    // Characters, not UTF-16 units: an emoji counts once.
    expect(propertiesBlocker({ ...orig, title: "😀".repeat(MAX_PROPERTY_CHARS) }, orig)).toBeNull();
  });

  it("sends only changed properties; an emptied field removes it", () => {
    expect(propertyChanges({ ...orig, author: "", subject: "Q3" }, orig)).toEqual({ author: "", subject: "Q3" });
    expect(propertyChanges(orig, orig)).toEqual({});
  });
});
