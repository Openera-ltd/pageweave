// Read aloud bar (PRD 09): plays the document paragraph by paragraph with the computer's voice,
// highlighting what is heard from the audio's own position (word or sentence timings from the
// speech engine, never a timer). Pause/resume, previous/next sentence, speed and voice; scrolling
// by hand stops following until "Follow" is pressed.

import { useCallback, useEffect, useRef, useState } from "react";
import { Crosshair, Pause, Play, SkipBack, SkipForward, X } from "lucide-react";
import { useApi } from "../api";
import { WebAudioPlayer } from "../webAudio";
import { highlightAt, paragraphs, READ_RATES, segmentFrom, sentenceStep, wordAt, type Segment } from "../readAloud";
import { toPwError, type ReadingPage, type SearchRect, type SpeechInfo, type Spoken } from "../types";

/** What playback needs from an audio element (replaced in tests). */
export interface AudioLike {
  src: string;
  currentTime: number;
  readonly paused: boolean;
  play(): Promise<void>;
  pause(): void;
  onended: (() => void) | null;
  onerror: (() => void) | null;
}

export interface ReadStart {
  page: number;
  /** Character offset in the page's reading text; null = the start of the page. */
  at: number | null;
  /** A selection rectangle to start from (its first word), resolved once the text is loaded. */
  rect?: SearchRect;
  nonce: number;
}

type Status = "starting" | "loading" | "playing" | "paused" | "finished" | "error";

interface Props {
  docId: number;
  pageCount: number;
  start: ReadStart;
  voice: string | null;
  rate: number;
  onSettings: (voice: string | null, rate: number) => void;
  onHighlight: (h: { page: number; rects: SearchRect[] } | null) => void;
  follow: boolean;
  onFollow: () => void;
  onClose: () => void;
  audioFactory?: () => AudioLike;
  /** Calls `cb` on the next display frame (replaced in tests). */
  frame?: (cb: () => void) => number;
  cancelFrame?: (id: number) => void;
}

interface Current {
  page: ReadingPage;
  seg: Segment;
  spoken: Spoken;
}

const key = (s: Segment, voice: string | null, rate: number) => `${s.page}:${s.start}:${voice ?? ""}:${rate}`;

export function ReadAloudBar({ docId, pageCount, start, voice, rate, onSettings, onHighlight, follow, onFollow, onClose, audioFactory, frame, cancelFrame }: Props) {
  const api = useApi();
  const [info, setInfo] = useState<SpeechInfo | null>(null);
  const [status, setStatus] = useState<Status>("starting");
  const [error, setError] = useState<{ message: string; retry: (() => void) | null } | null>(null);
  const [where, setWhere] = useState<number | null>(null);

  const audio = useRef<AudioLike | null>(null);
  const token = useRef(0);
  const pages = useRef(new Map<number, Promise<ReadingPage>>());
  const spokenCache = useRef(new Map<string, Promise<Spoken>>());
  const current = useRef<Current | null>(null);
  const raf = useRef(0);
  // A saved voice that is no longer installed falls back to the default voice.
  const known = (v: string | null, i: SpeechInfo | null) => (v && i && !i.voices.some((x) => x.id === v) ? null : v);
  const settings = useRef({ voice, rate });
  settings.current = { voice: known(voice, info), rate };
  const highlightRef = useRef(onHighlight);
  highlightRef.current = onHighlight;

  const nextFrame = frame ?? ((cb: () => void) => requestAnimationFrame(cb));
  const stopFrame = cancelFrame ?? ((id: number) => cancelAnimationFrame(id));

  const getAudio = (): AudioLike => {
    if (!audio.current) audio.current = audioFactory ? audioFactory() : new WebAudioPlayer();
    return audio.current;
  };

  const loadPage = useCallback(
    (page: number) => {
      let p = pages.current.get(page);
      if (!p) {
        p = api.readingText(docId, page);
        pages.current.set(page, p);
        p.catch(() => pages.current.delete(page));
      }
      return p;
    },
    [api, docId],
  );

  const speak = useCallback(
    (s: Segment) => {
      const { voice: v, rate: r } = settings.current;
      const k = key(s, v, r);
      let p = spokenCache.current.get(k);
      if (!p) {
        p = api.speakText(s.text, v, r);
        spokenCache.current.set(k, p);
        p.catch(() => spokenCache.current.delete(k));
        // Bounded: keep the last few segments only.
        if (spokenCache.current.size > 6) spokenCache.current.delete(spokenCache.current.keys().next().value!);
      }
      return p;
    },
    [api],
  );

  /** The first segment with something to say at or after (page, at), loading pages as needed. */
  const findFrom = useCallback(
    async (page: number, at: number): Promise<{ page: ReadingPage; seg: Segment } | null> => {
      for (let p = page, a = at; p < pageCount; p++, a = 0) {
        const text = await loadPage(p);
        const found = segmentFrom(text, a);
        if (found) return { page: text, seg: found.segment };
      }
      return null;
    },
    [loadPage, pageCount],
  );

  const stopTicking = () => {
    stopFrame(raf.current);
    raf.current = 0;
  };

  const tick = useCallback(() => {
    const c = current.current;
    const a = audio.current;
    if (!c || !a) return;
    const h = highlightAt(c.page, c.seg, c.spoken, a.currentTime * 1000);
    highlightRef.current(h ? { page: c.seg.page, rects: h.rects } : null);
    if (h) setWhere(h.start);
    if (!a.paused) raf.current = nextFrame(tick);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const fail = useCallback((message: string, retry: (() => void) | null) => {
    stopTicking();
    setStatus("error");
    setError({ message, retry });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** Start reading at (page, at), from `atMs` into that segment's audio. */
  const playFrom = useCallback(
    async (page: number, at: number, atMs = 0) => {
      const t = ++token.current;
      stopTicking();
      audio.current?.pause();
      setError(null);
      setStatus("loading");
      let found: { page: ReadingPage; seg: Segment } | null;
      try {
        found = await findFrom(page, at);
      } catch (e) {
        if (t === token.current) fail(toPwError(e).message, () => void playFrom(page, at, atMs));
        return;
      }
      if (t !== token.current) return;
      if (!found) {
        current.current = null;
        highlightRef.current(null);
        setStatus("finished");
        return;
      }
      let spoken: Spoken;
      try {
        spoken = await speak(found.seg);
      } catch (e) {
        if (t === token.current) fail(toPwError(e).message, () => void playFrom(page, at, atMs));
        return;
      }
      if (t !== token.current) return;
      current.current = { ...found, spoken };
      const a = getAudio();
      a.onended = () => {
        if (t !== token.current) return;
        const c = current.current;
        if (c) void playFrom(c.seg.page, c.seg.end + 1);
      };
      a.onerror = () => t === token.current && fail("The speech could not be played.", () => void playFrom(page, at, atMs));
      a.src = spoken.audio;
      a.currentTime = atMs / 1000;
      try {
        await a.play();
      } catch {
        if (t === token.current) fail("Sound could not be played. Check that an audio output device is connected.", () => void playFrom(page, at, atMs));
        return;
      }
      if (t !== token.current) return;
      setStatus("playing");
      raf.current = nextFrame(tick);
      // Prepare the next paragraph while this one plays.
      findFrom(found.seg.page, found.seg.end + 1)
        .then((n) => {
          if (n && t === token.current) void speak(n.seg).catch(() => undefined);
        })
        .catch(() => undefined);
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [findFrom, speak, fail, tick],
  );

  // The engine and its voices, once.
  useEffect(() => {
    let live = true;
    api
      .speechInfo()
      .then((i) => live && setInfo(i))
      .catch((e) => live && fail(toPwError(e).message, null));
    return () => {
      live = false;
    };
  }, [api, fail]);

  // Each start request (command, selection) begins reading.
  useEffect(() => {
    if (!info) return;
    let live = true;
    (async () => {
      let at = start.at ?? 0;
      if (start.rect) {
        try {
          const text = await loadPage(start.page);
          at = wordAt(text, start.rect) ?? 0;
        } catch {
          at = 0;
        }
      }
      if (live) void playFrom(start.page, at);
    })();
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [info, start.nonce]);

  // Stop everything when the bar goes away.
  useEffect(
    () => () => {
      token.current++;
      stopTicking();
      audio.current?.pause();
      highlightRef.current(null);
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  const toggle = () => {
    const a = audio.current;
    if (status === "playing" && a) {
      a.pause();
      stopTicking();
      setStatus("paused");
    } else if (status === "paused" && a) {
      const t = token.current;
      a.play().then(
        () => {
          if (t !== token.current) return;
          setStatus("playing");
          raf.current = nextFrame(tick);
        },
        () => fail("Sound could not be played. Check that an audio output device is connected.", toggle),
      );
    } else if (status === "finished") {
      void playFrom(0, 0);
    }
  };

  const step = (dir: 1 | -1) => {
    const c = current.current;
    const a = audio.current;
    if (!c || !a) return;
    const ms = a.currentTime * 1000;
    const target = sentenceStep(c.spoken.sentences, ms, dir);
    if (target !== null) {
      a.currentTime = target / 1000;
      tick();
      return;
    }
    if (dir > 0) {
      void playFrom(c.seg.page, c.seg.end + 1);
      return;
    }
    // The paragraph before this one (on this page or the previous page with text).
    void (async () => {
      for (let p = c.seg.page; p >= 0; p--) {
        const text = await loadPage(p).catch(() => null);
        if (!text) return;
        const ps = paragraphs(text).filter((s) => p < c.seg.page || s.end <= c.seg.start);
        const last = ps[ps.length - 1];
        if (last) return playFrom(p, last.start);
      }
      return playFrom(c.seg.page, c.seg.start);
    })();
  };

  /** Speed or voice changed: say the current sentence again with the new setting. */
  const restartHere = (v: string | null, r: number) => {
    onSettings(v, r);
    settings.current = { voice: v, rate: r };
    const c = current.current;
    if (!c || status === "finished") return;
    const ms = (audio.current?.currentTime ?? 0) * 1000;
    const marks = c.spoken.sentences;
    let i = -1;
    for (let k = 0; k < marks.length; k++) if (marks[k]!.start_ms <= ms) i = k;
    void playFrom(c.seg.page, c.seg.start + (i >= 0 ? marks[i]!.start : 0));
  };

  const busy = status === "loading" || status === "starting";
  const label =
    status === "error"
      ? (error?.message ?? "Reading stopped")
      : status === "finished"
        ? "Finished reading"
        : busy
          ? "Preparing…"
          : status === "paused"
            ? "Paused"
            : `Reading page ${(current.current?.seg.page ?? 0) + 1}`;
  const canStep = (status === "playing" || status === "paused") && current.current !== null;
  const precision = info?.precision === "word" ? "Highlights each word as it is spoken" : "Highlights each sentence as it is spoken";

  return (
    <div
      className="readbar"
      role="region"
      aria-label="Read aloud"
      data-testid="readbar"
      data-status={status}
      data-where={where ?? undefined}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          onClose();
        }
      }}
    >
      <button type="button" className="btn icon" data-testid="read-prev" aria-label="Previous sentence" title={canStep ? "Previous sentence" : "Previous sentence — not reading yet"} aria-disabled={!canStep || undefined} onClick={() => canStep && step(-1)}>
        <SkipBack />
      </button>
      <button
        type="button"
        className="btn icon"
        data-testid="read-toggle"
        aria-label={status === "playing" ? "Pause reading" : status === "finished" ? "Read again from the start" : "Resume reading"}
        title={status === "playing" ? "Pause" : status === "finished" ? "Read again from the start" : busy ? "Resume — preparing speech" : status === "error" ? "Resume — reading stopped" : "Resume"}
        aria-disabled={busy || status === "error" || undefined}
        onClick={() => !busy && status !== "error" && toggle()}
      >
        {status === "playing" ? <Pause /> : <Play />}
      </button>
      <button type="button" className="btn icon" data-testid="read-next" aria-label="Next sentence" title={canStep ? "Next sentence" : "Next sentence — not reading yet"} aria-disabled={!canStep || undefined} onClick={() => canStep && step(1)}>
        <SkipForward />
      </button>
      <span className={`status ${status === "error" ? "error" : ""}`} role="status" aria-live="polite" title={precision}>
        {label}
      </span>
      {status === "error" && error?.retry && (
        <button type="button" className="btn" data-testid="read-retry" onClick={() => error.retry?.()}>
          Try again
        </button>
      )}
      {!follow && status !== "finished" && status !== "error" && (
        <button type="button" className="btn" data-testid="read-follow" title="Scroll with the reading again" onClick={onFollow}>
          <Crosshair aria-hidden="true" /> Follow
        </button>
      )}
      <label className="sr-only" htmlFor="read-rate">
        Reading speed
      </label>
      <select id="read-rate" className="select" title="Reading speed" value={String(rate)} disabled={!info} onChange={(e) => restartHere(voice, Number(e.target.value))}>
        {(READ_RATES as readonly number[]).includes(rate) ? null : <option value={String(rate)}>{rate}×</option>}
        {READ_RATES.map((r) => (
          <option key={r} value={String(r)}>
            {r}×
          </option>
        ))}
      </select>
      <label className="sr-only" htmlFor="read-voice">
        Voice
      </label>
      <select
        id="read-voice"
        className="select"
        title="Voice"
        value={voice && info?.voices.some((v) => v.id === voice) ? voice : ""}
        disabled={!info || info.voices.length === 0}
        onChange={(e) => restartHere(e.target.value || null, rate)}
      >
        <option value="">{info ? `Default (${info.voices.find((v) => v.default)?.name ?? info.voices[0]?.name ?? "none"})` : "Default voice"}</option>
        {info?.voices.map((v) => (
          <option key={v.id} value={v.id}>
            {v.name} — {v.language}
          </option>
        ))}
      </select>
      <button type="button" className="btn icon" data-testid="read-stop" aria-label="Stop reading" title="Stop reading (Esc)" onClick={onClose}>
        <X />
      </button>
    </div>
  );
}
