import { describe, expect, it } from "vitest";
import { protectBlocker } from "./components/ProtectDialog";

describe("protect password rules (mirror pw-core)", () => {
  it("needs a confirmed password", () => {
    expect(protectBlocker("", "", "")).toBe("Enter a password.");
    expect(protectBlocker("abc", "abd", "")).toBe("The passwords do not match.");
    expect(protectBlocker("abc", "abc", "")).toBeNull();
  });
  it("limits length in bytes, not characters", () => {
    const long = "é".repeat(64); // 128 bytes in UTF-8
    expect(protectBlocker(long, long, "")).toBe("Passwords can be at most 127 bytes long.");
    expect(protectBlocker("é".repeat(63), "é".repeat(63), "")).toBeNull();
  });
  it("keeps the permissions password different", () => {
    expect(protectBlocker("same", "same", "same")).toBe("Use a different permissions password, or leave it empty.");
    expect(protectBlocker("open", "open", "owner")).toBeNull();
  });
});
