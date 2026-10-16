# Global Claude Code Preferences

These general preferences load in every Claude Code session for this repository (this rule file has no `paths` filter). To apply them across other projects too, merge them into `~/.claude/CLAUDE.md`. PageWeave-specific rules belong in the repository's `CLAUDE.md` and `docs/` files.

- Produce production-quality work, not demos dressed up as finished products.
- Never claim something works unless you verified it. Never claim tests passed unless they actually ran and passed.
- Every visible control must have real behavior and tests, or be intentionally disabled with a clear reason. No dead buttons, fake progress, placeholders disguised as features, or silent failures.
- Inspect the existing project and preserve working behavior before editing. Avoid unrelated rewrites and destructive commands. Review the final diff.
- Work in complete, testable increments. Define acceptance criteria, test the happy path and failure paths, run regression tests, launch the actual app where possible, and disclose blockers honestly.
- Prioritize UX quality, speed, accessibility, privacy, maintainability, and recovery—not merely code generation or compiling.
- Make interfaces compact, coherent, polished, responsive, and visually intentional. Avoid generic AI-looking UI, oversized cards, random emoji, unnecessary explanatory paragraphs, and decorative controls that do nothing.
- Use purposeful smooth animations, never let them delay feedback, and respect reduced-motion preferences.
- Measure performance on actual target hardware. Do not call a product fast based on source code or a development preview alone.
- Do not guess facts about the repository or product. Search the code and docs first. Mark unresolved decisions instead of silently inventing answers.
- When corrected, do not repeat the same mistake. Update the proper persistent instruction/spec and add a regression test when possible.
- Keep documentation honest and synchronized with implementation. Final reports must list actual verification performed, failures, limitations, and blockers.
