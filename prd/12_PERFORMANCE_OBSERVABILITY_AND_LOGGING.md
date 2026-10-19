# PRD 12 — Performance, Logging, Telemetry and Ten-Minute Watchdog

## Instrumentation
For each operation record start/queue/phase/first visible result/completion/cancellation/validation/commit timestamps. Record operation ID, correlation ID, input size/page count where safe, output size, build/engine/model/runtime version, CPU/GPU mode, duration, throughput, peak RAM/VRAM when available, warnings, terminal status and result validation. Never log content by default.

Metrics: cold/warm app startup, idle/peak RAM, CPU/GPU, UI event latency, frame-time distribution, scroll/zoom smoothness, cache size, memory growth; PDF first page/render/search/thumbnail timing; conversion/OCR/translation/design reconstruction pages/minute, time/page, throughput, quality, resource peaks, retries/cancel latency; TTS time to first audio, real-time factor, alignment quality, playback dropout/export throughput; resource download bytes/speed/retry/checksum/install/disk/checkpoint age; jobs queue depth, hung worker/stuck-job age/crash/retry/duplicate count.

## Logging rules
Structured timestamped JSONL/equivalent: UTC time, severity, component, operation/job ID, event, build/commit and safe diagnostics. Default INFO; opt-in DEBUG/TRACE. Bounded rotation by age/size/count; tolerate corruption/disk-full; no synchronous heavy logging on UI thread. Never record full document text/images, voice recordings, raw prompts, API keys, bearer tokens or signed URLs. Paths should be redacted or privacy-safe. Provide previewable/exportable diagnostic bundle excluding documents and credentials. Track tracing overhead.

## Every-10-minute watchdog
While supervised dev/build/training work is active, configure a real watchdog to run every 10 minutes, not just document it. Use Windows Task Scheduler for local development and/or CI scheduling; a remote CI schedule is best-effort, not guaranteed exact timing. The installed trigger must be test-proven.

Every run checks:
1. latest build/compiler/test/CI status and recent regression failures;
2. actual application process health, crashes, hangs and repeated errors;
3. performance sample/budget breach where safe;
4. CPU/RAM/VRAM/disk, disk growth, logs/cache, worker count;
5. training heartbeat, last checkpoint, step/progress, utilization and NaN/OOM failure if active;
6. resource installer/download health and checksum failures;
7. branch/commit/build, repository state, security/secret/dependency scan status;
8. duplicate running tasks, task journal and blockers.

## Watchdog safeguards
- Deterministic scripts and structured status first. Lock/lease/idempotency prevents overlapping checks; timeouts bound duration.
- Collect evidence and alert on failures. Deduplicate same signature; do not create one issue per loop. Safe auto-repair must be explicitly allowlisted. No blind delete, credential rotation, dependency upgrade, model restart, bucket write or release.
- If optional AI review is used, make it a bounded review of new evidence only. Do not invoke a model every ten minutes without a change. Track tokens/cost; no recursive agent loop.
- If PC sleeps/offline/session stops, a trigger cannot run. Record missed check and run a safe catch-up after wake if supported. Never claim checks ran when not executed.

## Performance methodology
Baseline before optimization. Measure p50/p95/p99. UI must remain responsive while heavy tasks run; async syntax alone does not offload CPU work. Investigate UI stalls ≥50 ms. Use virtualized page thumbnails/lists, render visible pages first, bound caches/concurrency, avoid repetitive parsing and giant copies. Test CPU-only, low RAM, integrated GPU, high DPI, slow drive, thermal throttling, background load and long sessions. Optimization must preserve output quality.

## Watchdog acceptance
Prove scheduled execution, timestamped output, injected test failure detection, performance budget breach detection, deduplication, overlap prevention, restart recovery, disk-full behavior, privacy and no destructive action by default. Save logs/results as artifacts. A requirement without a tested trigger is not complete.
