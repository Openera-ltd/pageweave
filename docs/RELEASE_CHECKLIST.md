# PageWeave Release Checklist — Mandatory Gate

A release is not shippable until each applicable item is checked with real evidence. Unknown or untested is not “pass.”

## Scope and implementation
- [ ] All committed release-scope requirements map to implemented functionality and tests.
- [ ] All open decisions affecting this release are resolved or explicitly scoped out.
- [ ] No dead controls, fake progress, placeholder success, unfinished core paths, or silent failure remain.
- [ ] No unrelated user changes are lost; diff has been reviewed.

## Build and tests
- [ ] Clean production build passes on the intended toolchain.
- [ ] Format, lint, type/static analysis, and security/dependency checks pass.
- [ ] Unit, integration, regression, and end-to-end tests pass.
- [ ] Actual packaged Windows app has been launched and core flows exercised.
- [ ] Required error, cancellation, retry, recovery, permissions, and low-resource tests pass.
- [ ] Accessibility and shortcut tests pass for release scope.

## User experience
- [ ] Controls have real behavior, clear states, correct feedback, and accessible names.
- [ ] Context menus and selection-aware actions behave correctly.
- [ ] Theme, density, shortcuts, reduced motion, focus, dialogs, and settings behave as specified.
- [ ] Screenshots/layout checks show no clipping, overlap, or critical contrast issue on supported sizes/scales.
- [ ] Animations are purposeful, interruptible, and not blocking; smoothness is measured where relevant.

## Correctness, privacy, security
- [ ] Output files open and contain the expected content/structure for the tested cases.
- [ ] Document fidelity limitations are surfaced; redaction is permanent and independently checked.
- [ ] Originals are preserved by default; overwrite/recovery behavior is verified.
- [ ] Paths, documents, API/MCP inputs, downloads, temporary files, and secrets have been reviewed.
- [ ] External data transfer is disclosed and requires the intended authorization.
- [ ] Dependency/model provenance, vulnerabilities, license terms, and required notices are accounted for.

## Resource setup and offline behavior
- [ ] First-run download/install flow tested on a clean machine.
- [ ] Resume/retry/restart, corrupted partials, integrity failure, low disk, and no network tested.
- [ ] Required resources pass integrity and smoke checks.
- [ ] Claimed core offline workflows were tested with networking disabled.
- [ ] Failed or missing resources are reported accurately without claiming setup complete.

## Performance, packaging, and docs
- [ ] Relevant startup/UI/render/job/memory benchmarks meet documented budgets or regressions are approved and documented.
- [ ] Minimum/recommended hardware claims correspond to actual test evidence.
- [ ] Tested artifact is byte-identical to the proposed release artifact or the release artifact is retested.
- [ ] Clean install, upgrade/restart, runtime discovery, and user-data preservation policy are tested.
- [ ] README and documentation match shipped capabilities, setup, limitations, privacy, supported formats/languages, and tests.
- [ ] Release notes list actual changes, limitations, and known issues truthfully.

## Final status
- [ ] **PASS** — all applicable required items have evidence.
- [ ] **BLOCKED** — one or more release-blocking items fail or could not be tested.

Release owner/reviewer: ____________________
Artifact/version: ____________________
Test environment/evidence: ____________________
Known limitations: ____________________
