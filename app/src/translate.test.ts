import { describe, expect, it } from "vitest";
import { languageName, languages, megabytes, parseGlossary, route } from "./translate";

const d = (source: string, target: string, installed = true, bytes = 111_000_000) => ({ source, target, installed, bytes });
const dirs = [d("en", "de"), d("de", "en"), d("en", "fr", false), d("fr", "en"), d("en", "it", false)];

describe("translation choices", () => {
  it("lists every offered language, English first", () => {
    expect(languages(dirs)).toEqual(["en", "fr", "de", "it"]);
    expect(languageName("de")).toBe("German");
    expect(languageName("xx")).toBe("xx");
    expect(megabytes(110_500_000)).toBe("111 MB");
  });

  it("routes directly or through English and lists what needs downloading", () => {
    expect(route(dirs, "en", "de")).toEqual({ steps: [dirs[0]], needs: [], viaEnglish: false });
    expect(route(dirs, "de", "fr")).toEqual({ steps: [dirs[1], dirs[2]], needs: [dirs[2]], viaEnglish: true });
    expect(route(dirs, "en", "en")).toBe("Choose two different languages.");
    expect(route(dirs, "it", "en")).toBe("Translating from Italian to English is not available.");
  });
});

describe("glossary", () => {
  it("reads term = translation lines, skips blanks and comments, and names the line that is wrong", () => {
    expect(parseGlossary("")).toEqual([]);
    expect(parseGlossary("# ours\r\n\n fixture page = Testseite \nfox=Wolf")).toEqual([
      { source: "fixture page", target: "Testseite" },
      { source: "fox", target: "Wolf" },
    ]);
    expect(parseGlossary("fox = Wolf\nno sign")).toBe('Glossary line 2: write it as "term = translation".');
    expect(parseGlossary("= Wolf")).toBe('Glossary line 1: write it as "term = translation".');
    expect(parseGlossary(`${"a".repeat(201)} = b`)).toBe("Glossary line 1 is too long (at most 200 characters a side).");
    expect(parseGlossary(Array.from({ length: 1001 }, (_, i) => `t${i} = u`).join("\n"))).toBe("A glossary can have at most 1000 lines.");
  });
});
