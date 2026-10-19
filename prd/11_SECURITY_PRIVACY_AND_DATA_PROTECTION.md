# PRD 11 — Security, Privacy, Trust and Data Protection

## Threat model
Assume malformed or hostile PDFs/Office files, archives, SVGs, embedded attachments, scripts/links, prompt injection, confidential text, credentials and malicious metadata. External AI/API/MCP inputs and responses are also untrusted.

## Safe file processing
Use maintained parsers and record versions/advisories. Sandbox workers where possible. Bound file size, page count, image pixels, recursion, CPU, memory and time. Never execute PDF JavaScript, Office macros, embedded executables or launch actions automatically. Safe URL handling on user-initiated links. Prevent path traversal, symlink escape, zip slip, temp-file races, command injection, DLL path hijacking and unsafe deserialization. Validate file magic and parser result, not extension alone.

## Network/privacy boundaries
Documents, OCR text, tables, voice samples and assets remain local unless a clear per-operation consent allows external processing. Model update downloads must not contain user documents. Explain what content is sent to an external AI provider, minimize scope and allow local-only mode. Logs/diagnostic bundles exclude contents and secrets by default and are user-reviewable. No user files are used for training by default.

## Secrets
No raw tokens/credentials in source, config defaults, prompts, notebooks, screenshots, logs, crash reports, datasets, CI artifacts or Git. Use a credential manager or protected runtime secrets; least privilege and separate development/training/release credentials. A detected exposure blocks publication and triggers revoke/rotate/history-scan steps. End-user application must never contain a Cloudflare admin token.

## Redaction and MCP/API
Permanent redaction writes a new output by default and is validated with independent extraction. Sensitive-data detection is advisory unless the user authorizes a specific workflow. Local API binds loopback by default; network access requires opt-in and authentication. Typed MCP tools restrict allowed folders/operations, request size, rate and destructive side effects. No arbitrary shell or broad filesystem access. Document prompt injection cannot grant permissions or reveal secrets.

## Integrity and trust
Verify downloads by checksum and signature when trustworthy signing is available. Protect update/model manifests and allow rollback. Distinguish digital signature cryptographic validity from identity trust/revocation. Offline validation states its limitations.

## Security acceptance
Threat-model review; secret scan; dependency/SBOM/license scan; hostile PDF/Office corpus; path-traversal and command tests; redaction verification; network inspection proving local operations are offline; API/MCP scope tests; privacy review of logs; signed/checksummed artifacts. Critical security failures block shipping.
