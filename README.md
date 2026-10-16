# PageWeave

PageWeave is planned as a free/open-source Windows desktop document studio. See `docs/PRODUCT_VISION.md` for the intended scope.

## Status

**Pre-implementation.** This repository currently holds engineering instructions and specifications only. No application code, build, tests, or packaged release exist yet, and nothing described in `docs/` should be read as implemented functionality.

## Engineering instructions

- `CLAUDE.md` — mandatory project rules for every Claude Code session.
- `docs/INDEX.md` — index of the product/engineering knowledge base (vision, architecture, interaction contract, UX system, AI/resources, quality gates, test matrix, release checklist, decisions).
- `.claude/rules/` — scoped rules that Claude Code loads for matching files, plus general preferences that load in every session.

Instructions alone do not enforce quality. CI and release automation for the gates in `docs/QUALITY_GATES.md` will be added once the toolchain is chosen and recorded in `docs/DECISIONS.md`.

## License

MIT — see `LICENSE`. Third-party dependency and model notices will be added here and in required notice files as dependencies are adopted.
