# PRD 19 — Continuous Build/Test/Fix Loop and Durable Project Memory

## Intended working behavior
Claude Code should keep working through the approved backlog while its session and compute are available; it must not stop at a plan, screenshot, prototype, compile or one green unit suite. It must actively reproduce defects, fix root causes, add regressions, profile and test the packaged app. It must never claim it can run forever after the host/session has stopped.

## Durable journal
Maintain `docs/WORK_JOURNAL.md`: timestamp, branch/commit, objective and requirement IDs; completed work; changed files; exact build/test commands/results; latest performance and quality delta; active process/run IDs; latest durable model checkpoint; next small task and resume command; blockers; budget and recovery state. Update after every meaningful work unit and before restart/high-risk command. Never put secrets or private content in journal.

## Required loop
1. Read `CLAUDE.md`, `prd/INDEX.md`, master/relevant PRDs, decisions, journal, Git status/diff and monitor reports.
2. Pick highest-risk/highest-value unverified requirement that can progress.
3. Define acceptance and failure tests before/with code.
4. Implement one complete, reviewable slice.
5. Run targeted/unit/integration tests and real-app/package tests appropriate to scope.
6. Actively try edge cases, failure/cancel/restart and malicious inputs.
7. Profile UI speed, resource use and output quality; compare to baseline.
8. Fix root cause and add regression test; never suppress/weaken failing checks to make green.
9. Audit diff for data loss, unrelated changes, dead controls, secrets, unlicensed assets, placeholders and misleading docs.
10. Update traceability/docs/journal/artifacts and commit coherent units if repository policy allows.
11. Repeat until all agreed gates pass or an actual external/human blocker prevents progress.

## Safe automation
Build/test/profile/read-only diagnostic checks can be automated within limits. Bounded retries with exponential backoff for transient downloads. Resume only from verified checkpoints. Use process locks/idempotency. Do not run duplicate trainings/builds or poll APIs constantly. Don't kill a live job without determining its state.

## Human approval gates
Public release/deployment; paid cloud spend beyond approved cap; deleting data/buckets/checkpoints; changing token permissions; user documents sent outside device; destructive user-document changes; publishing license-unclear models/data; license/architecture/scope pivots. Routine tests and safe bug fixes don't require repeated confirmation where task/branch scope is clear.

## Never do
Unbounded recursive agent/subagent calls; unlimited token/GPU/bandwidth/storage use; duplicate experiments; silent failure; invented test results; automatic destructive repair; weakened tests; auto-publish from watchdog; or pretend background work continues after process exit. Persist enough context to resume with minimal duplicated effort.

## Stop condition
Finish only when mandatory acceptance/release gates pass, or if an external blocker makes meaningful work impossible. Then status is **BLOCKED — NOT PRODUCTION READY**, with evidence and next required action. Do not endlessly loop on external blockers; continue independent work where possible. The objective is persistence with checkpoints, not an impossible guarantee of infinite runtime.
