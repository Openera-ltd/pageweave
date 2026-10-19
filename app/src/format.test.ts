import { describe, expect, it } from "vitest";
import { formatBytes, sizeChange } from "./format";

describe("format", () => {
  it("formats sizes", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2.0 KB");
    expect(formatBytes(3 * 1024 * 1024)).toBe("3.0 MB");
  });
  it("describes a size change without overstating it", () => {
    expect(sizeChange(2048, 1024)).toBe("2.0 KB → 1.0 KB (50% smaller)");
    expect(sizeChange(1000, 999)).toBe("1000 B → 999 B (less than 1% smaller)");
    expect(sizeChange(1000, 661)).toBe("1000 B → 661 B (33% smaller)");
  });
});
