---
paths:
  - "**/*.{rs,cpp,cxx,cc,h,hpp,c}"
---
# Native core and performance rules

- Read `docs/ARCHITECTURE.md`, `docs/QUALITY_GATES.md`, and `docs/TEST_MATRIX.md` before changing Rust/C/C++ core code.
- Define ownership, lifetimes, encoding, error mapping, threading, cancellation, and FFI boundaries explicitly. Do not share ambiguous borrowed memory across language boundaries.
- Avoid UI-thread I/O, parsing, rendering, inference, and CPU-heavy work. Bound worker concurrency and memory/cache sizes.
- Preserve input files; use safe temporary output and atomic finalization where supported. Never silently overwrite.
- Add tests for state transitions, malformed input, errors, cancellation, retry/recovery, large inputs, and resource exhaustion as applicable.
- Profile before optimizing; report measured results and exact environment. Never infer performance solely from source inspection.
