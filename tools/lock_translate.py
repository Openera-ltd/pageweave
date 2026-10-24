#!/usr/bin/env python3
"""Pin the translation directions of resources/translate.models.json in resources/translate.lock.json.

Usage: python3 -I tools/lock_translate.py MANIFEST.json

MANIFEST.json is the manifest the Translation models workflow publishes with the converted files
(sizes and SHA-256 of each file as served). Every direction listed for a model becomes one lock
entry pointing at that model's shared folder, its release files and its target-language token.
Exit codes: 0 ok, 2 bad input (a listed model is missing from the manifest, or the manifest is for
another release).
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SPEC = ROOT / "resources" / "translate.models.json"
LOCK = ROOT / "resources" / "translate.lock.json"
FILES = ("encoder.onnx", "decoder.onnx", "tokenizer.json", "config.json")
# Target-language tokens of the multilingual models (ISO 639-3).
TOKENS = {"en": ">>eng<<", "de": ">>deu<<", "fr": ">>fra<<", "es": ">>spa<<", "it": ">>ita<<", "pt": ">>por<<"}


def build(spec: dict, manifest: dict) -> list[dict]:
    if manifest.get("release") != spec["release"]:
        raise ValueError(f"manifest is for {manifest.get('release')!r}, not {spec['release']!r}")
    pairs = []
    seen = set()
    for m in spec["models"]:
        files = {}
        for f in FILES:
            asset = f"{m['name']}-{f}"
            if asset not in manifest["files"]:
                raise ValueError(f"{asset} is not published")
            files[f] = {"from": asset, **manifest["files"][asset]}
        for source, target in m["directions"]:
            if (source, target) in seen:
                raise ValueError(f"{source}-{target} is listed twice")
            seen.add((source, target))
            pairs.append(
                {
                    "source": source,
                    "target": target,
                    "model": m["repo"],
                    "repository": f"https://huggingface.co/{m['repo']}",
                    "commit": m["commit"],
                    "license": m["license"],
                    "folder": m["name"],
                    "prefix": TOKENS[target],
                    "url": manifest["url"],
                    "files": files,
                }
            )
    return pairs


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    spec = json.loads(SPEC.read_text(encoding="utf-8"))
    manifest = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    try:
        pairs = build(spec, manifest)
    except ValueError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    lock = json.loads(LOCK.read_text(encoding="utf-8"))
    lock["decision"] = spec["decision"]
    lock["license"] = (
        "Apache-2.0 (OPUS-MT tc-bible-big models) and CC-BY-4.0 (OPUS-MT tc-big gmw-gmw), Helsinki-NLP / "
        "University of Helsinki, Language Technology Research Group; converted to ONNX for PageWeave by "
        "tools/convert_translate.py. Attribution: Jörg Tiedemann et al., 'Democratizing neural machine "
        "translation with OPUS-MT', Language Resources and Evaluation 58, 2024; Tiedemann and Thottingal, "
        "'OPUS-MT — Building open translation services for the World', EAMT 2020."
    )
    lock["pairs"] = pairs
    LOCK.write_text(json.dumps(lock, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"pinned {len(pairs)} directions from {len(spec['models'])} models")
    return 0


if __name__ == "__main__":
    sys.exit(main())
