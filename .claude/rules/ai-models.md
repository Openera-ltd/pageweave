---
paths:
  - "**/*.{py,onnx,json,yaml,yml,toml}"
  - "**/*model*"
  - "**/*ocr*"
  - "**/*translation*"
  - "**/*speech*"
---
# AI/model and resource rules

- Read `docs/AI_AND_RESOURCES.md` before changing model selection, downloads, inference, language support, OCR, translation, TTS, voice cloning, or reconstruction.
- Do not bundle a general-purpose chat LLM. Specialist capabilities are required; external AI is optional orchestration only.
- Verify exact model/runtime versions, license/provenance, redistribution constraints, download sources, hashes/signatures, size, hardware use, and held-out quality before adoption.
- Test actual inference and representative outputs. A model loading successfully does not prove quality.
- Resource installation must support interruption, verified resume, retry, corruption detection, low disk, offline startup, and recovery. Never execute unverified downloads or claim a feature works when its resources are missing.
- Keep model names out of normal UI; technical inventory belongs in repository documentation/notices.
- Do not infer a language is supported or high-quality from a model card alone. Maintain measured per-language/direction coverage.
