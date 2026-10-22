import { describe, expect, it } from "vitest";
import { availability, languageName, languages } from "./translate";

const d = (source: string, target: string, installed = true) => ({ source, target, installed });
const dirs = [d("en", "de"), d("de", "en"), d("en", "fr"), d("fr", "en"), d("en", "it", false)];

describe("translation choices", () => {
  it("lists installed languages, English first", () => {
    expect(languages(dirs)).toEqual(["en", "fr", "de"]);
    expect(languageName("de")).toBe("German");
    expect(languageName("xx")).toBe("xx");
  });

  it("says whether a pair is available, through English, or why not", () => {
    expect(availability(dirs, "en", "de")).toBe(true);
    expect(availability(dirs, "de", "fr")).toBe("via-english");
    expect(availability(dirs, "en", "en")).toBe("Choose two different languages.");
    expect(availability(dirs, "en", "it")).toBe("Translating from English to Italian is not installed.");
    expect(availability(dirs, null, "de")).toBe(true);
    expect(availability(dirs, null, "it")).toBe("Nothing can be translated into Italian.");
  });
});
