// Playback of read-aloud speech through Web Audio (WebKitGTK's media element cannot play WAV data
// in every setup; Web Audio decodes it in both WebViews). The position comes from the audio
// context's clock, i.e. what has actually been played.

import type { AudioLike } from "./components/ReadAloudBar";

let shared: AudioContext | null = null;

function context(): AudioContext {
  if (!shared) shared = new AudioContext();
  return shared;
}

function bytesOf(dataUrl: string): ArrayBuffer {
  const comma = dataUrl.indexOf(",");
  const bin = atob(dataUrl.slice(comma + 1));
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out.buffer;
}

export class WebAudioPlayer implements AudioLike {
  onended: (() => void) | null = null;
  onerror: (() => void) | null = null;
  private url = "";
  private buffer: AudioBuffer | null = null;
  private decodedFor = "";
  private source: AudioBufferSourceNode | null = null;
  /** Seconds into the buffer where playback (re)started, and the context time it started at. */
  private offset = 0;
  private startedAt = 0;
  private playing = false;

  get src(): string {
    return this.url;
  }

  set src(v: string) {
    this.stopSource();
    this.url = v;
    this.offset = 0;
    this.playing = false;
  }

  get paused(): boolean {
    return !this.playing;
  }

  get currentTime(): number {
    if (!this.playing) return this.offset;
    const t = this.offset + (context().currentTime - this.startedAt);
    return this.buffer ? Math.min(t, this.buffer.duration) : t;
  }

  set currentTime(t: number) {
    const wasPlaying = this.playing;
    this.stopSource();
    this.offset = Math.max(0, t);
    this.playing = false;
    if (wasPlaying) void this.play().catch(() => this.onerror?.());
  }

  async play(): Promise<void> {
    const ctx = context();
    if (ctx.state === "suspended") await ctx.resume();
    if (this.decodedFor !== this.url || !this.buffer) {
      const url = this.url;
      const buf = await ctx.decodeAudioData(bytesOf(url));
      if (url !== this.url) return; // replaced meanwhile
      this.buffer = buf;
      this.decodedFor = url;
    }
    this.stopSource();
    const src = ctx.createBufferSource();
    src.buffer = this.buffer;
    src.connect(ctx.destination);
    src.onended = () => {
      // Only a natural end counts (stop() for pause/seek clears `source` first).
      if (this.source !== src) return;
      this.source = null;
      this.playing = false;
      this.offset = this.buffer?.duration ?? 0;
      this.onended?.();
    };
    const at = Math.min(this.offset, this.buffer.duration);
    this.source = src;
    this.startedAt = ctx.currentTime;
    this.offset = at;
    this.playing = true;
    src.start(0, at);
  }

  pause(): void {
    if (!this.playing) return;
    this.offset = this.currentTime;
    this.stopSource();
    this.playing = false;
  }

  private stopSource() {
    const s = this.source;
    this.source = null;
    if (s) {
      try {
        s.stop();
      } catch {
        // Already stopped.
      }
      s.disconnect();
    }
  }
}
