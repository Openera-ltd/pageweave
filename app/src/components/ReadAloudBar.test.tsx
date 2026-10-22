import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiContext, type Api } from "../api";
import type { ReadingPage, SearchRect, Spoken } from "../types";
import { ReadAloudBar, type AudioLike, type ReadStart } from "./ReadAloudBar";

const r = (left: number): SearchRect => ({ left, bottom: 700, right: left + 30, top: 710 });
const PAGES: ReadingPage[] = [
  // "One two. Three.\nFour"
  { page: 0, text: "One two. Three.\nFour", words: [{ start: 0, end: 3, rects: [r(0)] }, { start: 4, end: 8, rects: [r(40)] }, { start: 9, end: 15, rects: [r(80)] }, { start: 16, end: 20, rects: [r(120)] }] },
  { page: 1, text: "", words: [] },
  { page: 2, text: "Last page", words: [{ start: 0, end: 4, rects: [r(0)] }, { start: 5, end: 9, rects: [r(50)] }] },
];

function spokenFor(text: string): Spoken {
  // Fake timings: 100 ms per character, words at their offsets; sentences split at ". ".
  const words = [...text.matchAll(/\S+/g)].map((m) => ({ start: m.index!, end: m.index! + m[0].length, start_ms: m.index! * 100, end_ms: (m.index! + m[0].length) * 100 }));
  const sentences: Spoken["sentences"] = [];
  let s = 0;
  for (const m of text.matchAll(/\. /g)) {
    sentences.push({ start: s, end: m.index! + 1, start_ms: s * 100, end_ms: (m.index! + 1) * 100 });
    s = m.index! + 2;
  }
  sentences.push({ start: s, end: text.length, start_ms: s * 100, end_ms: text.length * 100 });
  return { audio: `data:audio/wav;base64,${btoa(text)}`, duration_ms: text.length * 100, sentences, words, precision: "word" };
}

class FakeAudio implements AudioLike {
  src = "";
  currentTime = 0;
  paused = true;
  onended: (() => void) | null = null;
  onerror: (() => void) | null = null;
  failPlay = false;
  async play() {
    if (this.failPlay) throw new Error("NotAllowedError");
    this.paused = false;
  }
  pause() {
    this.paused = true;
  }
}

function setup(opts: { start?: Partial<ReadStart>; noEngine?: boolean; voice?: string | null } = {}) {
  const audio = new FakeAudio();
  const frames: Array<() => void> = [];
  const speakText = vi.fn(async (text: string) => spokenFor(text));
  const api = {
    speechInfo: vi.fn(async () => {
      if (opts.noEngine) throw { category: "missing_resource", code: "speech.unavailable", message: "No speech voice is available on this computer.", action: "", retryable: false };
      return { name: "Test speech", precision: "word", voices: [{ id: "v1", name: "Voice One", language: "en-US", default: true }, { id: "v2", name: "Voice Two", language: "en-GB", default: false }] };
    }),
    readingText: vi.fn(async (_d: number, p: number) => PAGES[p]!),
    speakText,
  } as unknown as Api;
  const onHighlight = vi.fn();
  const onSettings = vi.fn();
  const onClose = vi.fn();
  const start: ReadStart = { page: 0, at: null, nonce: 1, ...opts.start };
  // Like the app: the saved choice comes back as the bar's props.
  function Host() {
    const [v, setV] = useState<{ voice: string | null; rate: number }>({ voice: opts.voice ?? null, rate: 1 });
    return (
      <ReadAloudBar
        docId={7}
        pageCount={3}
        start={start}
        voice={v.voice}
        rate={v.rate}
        onSettings={(voice, rate) => {
          onSettings(voice, rate);
          setV({ voice, rate });
        }}
        onHighlight={onHighlight}
        follow={true}
        onFollow={() => undefined}
        onClose={onClose}
        audioFactory={() => audio}
        frame={(cb) => frames.push(cb)}
        cancelFrame={() => undefined}
      />
    );
  }
  const view = render(
    <ApiContext.Provider value={api}>
      <Host />
    </ApiContext.Provider>,
  );
  const flush = async () => {
    await act(async () => {
      for (let i = 0; i < 10; i++) await Promise.resolve();
    });
  };
  /** Advance the audio to `ms` and run one display frame. */
  const at = async (ms: number) => {
    audio.currentTime = ms / 1000;
    await act(async () => {
      frames.splice(0).forEach((f) => f());
    });
  };
  return { audio, api, speakText, onHighlight, onSettings, onClose, view, flush, at };
}

const bar = () => screen.getByTestId("readbar");

afterEach(cleanup);

describe("Read aloud bar", () => {
  it("reads paragraphs in order, highlighting the word heard, across pages without text", async () => {
    const { audio, speakText, onHighlight, flush, at } = setup();
    await flush();
    expect(speakText).toHaveBeenCalledWith("One two. Three.", null, 1);
    expect(bar().dataset.status).toBe("playing");
    await at(450); // "two." starts at 400 ms
    expect(onHighlight).toHaveBeenLastCalledWith({ page: 0, rects: [r(40)] });
    expect(screen.getByRole("status").textContent).toBe("Reading page 1");
    // The next paragraph was prepared while this one played.
    expect(speakText).toHaveBeenCalledWith("Four", null, 1);
    await act(async () => audio.onended?.());
    await flush();
    expect(audio.src).toBe(spokenFor("Four").audio);
    await at(0);
    expect(onHighlight).toHaveBeenLastCalledWith({ page: 0, rects: [r(120)] });
    // Page 2 has no text: reading goes on with page 3, then finishes.
    await act(async () => audio.onended?.());
    await flush();
    expect(speakText).toHaveBeenLastCalledWith("Last page", null, 1);
    await act(async () => audio.onended?.());
    await flush();
    expect(bar().dataset.status).toBe("finished");
    expect(screen.getByRole("status").textContent).toBe("Finished reading");
    expect(onHighlight).toHaveBeenLastCalledWith(null);
  });

  it("starts from the first selected word", async () => {
    const { speakText, flush } = setup({ start: { page: 0, rect: { left: 82, bottom: 701, right: 100, top: 709 } } });
    await flush();
    await flush();
    expect(speakText).toHaveBeenCalledWith("Three.", null, 1);
  });

  it("pauses, resumes and steps between sentences", async () => {
    const { audio, flush, at } = setup();
    await flush();
    const toggle = screen.getByTestId("read-toggle");
    expect(toggle.getAttribute("aria-label")).toBe("Pause reading");
    await act(async () => fireEvent.click(toggle));
    expect(audio.paused).toBe(true);
    expect(bar().dataset.status).toBe("paused");
    expect(toggle.getAttribute("aria-label")).toBe("Resume reading");
    await act(async () => fireEvent.click(toggle));
    await flush();
    expect(audio.paused).toBe(false);
    await at(200);
    await act(async () => fireEvent.click(screen.getByTestId("read-next")));
    expect(audio.currentTime).toBeCloseTo(0.9); // "Three." starts at character 9
    await at(1500);
    await act(async () => fireEvent.click(screen.getByTestId("read-prev")));
    expect(audio.currentTime).toBeCloseTo(0); // within a second of its start: the sentence before
  });

  it("says the current sentence again with a new speed or voice, and saves the choice", async () => {
    const { speakText, onSettings, flush, at } = setup();
    await flush();
    await at(1000);
    await act(async () => {
      fireEvent.change(screen.getByLabelText("Reading speed"), { target: { value: "1.5" } });
    });
    await flush();
    expect(onSettings).toHaveBeenLastCalledWith(null, 1.5);
    expect(speakText).toHaveBeenCalledWith("Three.", null, 1.5);
    await act(async () => {
      fireEvent.change(screen.getByLabelText("Voice"), { target: { value: "v2" } });
    });
    await flush();
    expect(onSettings).toHaveBeenLastCalledWith("v2", 1.5);
    expect(speakText).toHaveBeenCalledWith("Three.", "v2", 1.5);
  });

  it("falls back to the default voice when the saved one is gone", async () => {
    const { speakText, flush } = setup({ voice: "removed-voice" });
    await flush();
    expect(speakText).toHaveBeenCalledWith("One two. Three.", null, 1);
    expect((screen.getByLabelText("Voice") as HTMLSelectElement).value).toBe("");
  });

  it("explains failures and retries", async () => {
    const { audio, flush } = setup({});
    audio.failPlay = true;
    await flush();
    expect(bar().dataset.status).toBe("error");
    expect(screen.getByRole("status").textContent).toContain("Sound could not be played");
    expect(screen.getByTestId("read-toggle").getAttribute("aria-disabled")).toBe("true");
    audio.failPlay = false;
    await act(async () => fireEvent.click(screen.getByTestId("read-retry")));
    await flush();
    expect(bar().dataset.status).toBe("playing");
  });

  it("says when there is no voice, and stops with Escape or the close button", async () => {
    const { onClose, flush } = setup({ noEngine: true });
    await flush();
    expect(screen.getByRole("status").textContent).toBe("No speech voice is available on this computer.");
    expect(screen.queryByTestId("read-retry")).toBeNull();
    fireEvent.keyDown(bar(), { key: "Escape" });
    fireEvent.click(screen.getByTestId("read-stop"));
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});
