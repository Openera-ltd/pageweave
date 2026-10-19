# PRD 15 — PDF Dataset Acquisition, Rights and Data Quality

## Mandatory rule
“Available on the internet” does not equal permission to train. Do not crawl arbitrary PDFs indiscriminately, bypass paywalls/authentication/access controls, ignore site terms or use private user documents by default. Use only material whose rights/terms permit collection, retention, preprocessing, intended training/evaluation and—where planned—model-weight redistribution/product use.

## Preferred sources
1. Verified public-domain material (jurisdiction and status recorded).
2. Open-licensed data whose exact terms permit the intended ML training/modification and model/product distribution.
3. Government/university/nonprofit collections with clear reuse/ML permissions.
4. Direct permission or user contribution with clear consent and documented terms.
5. Synthetic documents where appropriate and privacy safe.

Do not assume a paper repository means all papers can be used for any training purpose. Read exact terms per source/dataset/content. Ambiguous large-scale collection gets legal review or exclusion. Never evade restrictions.

## Ingestion process
- Source allowlist; per-source rate limits and terms checks; no access-control bypass; integrity/malware-safe download pipeline.
- Capture source URL/ID, creator/publisher, retrieved date, checksum, license ID/URI, terms version/evidence, intended purpose, commercial-use/weight-redistribution permission, transformation history, removal contact and reviewer.
- Quarantine unknown rights. Missing or incompatible rights state blocks training.
- Deduplicate exact/near copies. Detect PII, signatures, credentials, private IDs, confidential markings and minors' data; exclude/minimize/redact or require a documented legal basis/consent and safeguards.
- Isolated parse, enforce size/page/resource limits, remove unsafe active content. Keep originals separate from transformed OCR text/images/annotations.
- Respect takedown/withdrawal obligations, track lineage to derived datasets/checkpoints and document realistic limits of erasing training influence.

## Data quality/splits
Cover diverse languages, scripts, scan quality, fonts/layouts/document types without over-representing a single source. Ground-truth labels have version, annotator/license and uncertainty handling; use reviewer agreement where feasible. Split by source document, source/creator family and near-duplicate cluster; seal a blind test from training and iterative tuning. Store source hash, split, preprocessing, annotation and evaluation protocol.

## User privacy
No hidden user-document training. Never upload user PDFs to Kaggle/R2 unless the user deliberately contributes them under explicit lawful terms. Access restricted datasets; encrypt as appropriate; record access without contents; keep diagnostic logs free of text. Review memorization/privacy risk before publishing models.

## Release gate
Every training item has approved rights status; license and privacy review pass; duplicates/leakage checked; provenance and transformations are reproducible. If rights are unclear, exclude that PDF. Training amount never overrides rights obligations.
