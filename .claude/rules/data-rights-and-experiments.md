# Data rights and experiment safety

- Before acquisition/training, read the relevant PRDs (especially 14–16). Never assume a PDF is training-eligible just because it can be downloaded. Enforce source allowlist, rights/provenance manifest, privacy review and duplicate/leakage checks. Do not bypass access controls.
- Never put raw tokens or credentials, or screenshots revealing them, in chat, notebooks, source, logs, artifacts or Git. Use approved secret storage and least privilege; rotate immediately after exposure.
- Every training run is bounded, checkpointed, resumable, traceable to code/config/data manifests and evaluated against a baseline using an untouched holdout. Verify exact model-weight and dependency licenses.
