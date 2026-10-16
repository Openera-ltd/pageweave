# PRD 16 — Kaggle, GitHub, Cloudflare R2 and Credential Safety

## Principle
Credentials are secrets, not ordinary inputs. Never request, transcribe, store or echo raw token values in prompts, screenshots, markdown, source files, logs, crash reports, artifacts or Git. If the user offers a screenshot, require redaction of token values; prefer direct entry into provider UI/OS credential manager/CI secret store. If exposed, stop use, revoke/rotate, scan history and artifacts.

## GitHub
Use fine-grained repository-scoped/short-lived credentials with minimum permissions. Do not put PATs in remote URLs or plain `.git/config`; use secure CLI credential storage. No force-push/destructive history edits without approval. Protect branches with required tests where possible. Keep large weights/data out of Git; model metadata, scripts and manifests can be versioned, subject to rights.

## Cloudflare R2
Use R2 only for approved model/checkpoint/build artifacts, not user docs by default. Separate development/staging/release prefixes, least privilege, no public writes, retention/versioning consistent with recovery. Upload to temporary key, validate checksum/size, then promote manifest. Candidate/unapproved or rights-unclear artifacts cannot reach release prefix. Do not store tokens/signed URLs in logs. The end-user app must not need a Cloudflare admin token to edit local files.

## Kaggle
Official API/secret flow only. Keep keys out of notebook outputs, shell history, environment dumps and committed notebook state. Clean output before sharing. Credentials should be runtime-injected and restricted. Detect secret leaks in repo/history and build artifacts.

## Automated controls
`.gitignore` excludes env files, keys, credentials, private PDFs/datasets, voice profiles, user documents, logs and checkpoints, except deliberate versioned manifests/metadata. Add secret scanner, dependency/SBOM/license scanner and artifact allowlist in CI. On detection, redact report, stop publishing, identify type/provider without printing the value and give rotation/cleanup actions.

## Approval boundaries
Human approval before public release/deploy, new paid cloud spend beyond a set budget, deleting datasets/checkpoints/buckets, changing account permissions or using credentials for a new purpose. Approved read-only checks may run automatically. Never let “continue until ready” create uncontrolled bills or long-running jobs. Set ceilings for GPU time, storage, bandwidth and model API/token usage; pause at verified checkpoints if ceiling reached.
