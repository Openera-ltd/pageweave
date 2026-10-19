"""Training-data gate (PRD 15): a dataset split may be used for training only if every item carries
an approved rights record that permits training, and the split is not the sealed blind test.

Usage: python -m pwml.rights CORPUS_DIR SPLIT [SPLIT ...]   (exit 0 = trainable)
"""
from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

REQUIRED = ("id", "image", "image_sha256", "source", "rights")


class NotTrainable(Exception):
    pass


def check_trainable(corpus: Path, split: str, verify_files: bool = True) -> dict:
    d = corpus / split
    if (d / "SEALED").exists():
        raise NotTrainable(f"{split} is the sealed blind test split; it can be evaluated, never trained or tuned on")
    manifest = d / "manifest.jsonl"
    if not manifest.exists():
        raise NotTrainable(f"{split}: no manifest.jsonl, so no rights record; refusing to train")
    problems: list[str] = []
    n = 0
    for i, line in enumerate(manifest.read_text().splitlines(), 1):
        if not line.strip():
            continue
        n += 1
        try:
            item = json.loads(line)
        except json.JSONDecodeError:
            problems.append(f"line {i}: not JSON")
            continue
        missing = [k for k in REQUIRED if k not in item]
        if missing:
            problems.append(f"{item.get('id', i)}: missing {', '.join(missing)}")
            continue
        r = item["rights"]
        if r.get("status") != "approved":
            problems.append(f"{item['id']}: rights status is {r.get('status')!r} (quarantined)")
        if r.get("training") is not True:
            problems.append(f"{item['id']}: rights do not permit training")
        if verify_files:
            path = corpus / item["image"]
            if not path.exists():
                problems.append(f"{item['id']}: image missing")
            elif hashlib.sha256(path.read_bytes()).hexdigest() != item["image_sha256"]:
                problems.append(f"{item['id']}: image does not match its recorded hash")
        if len(problems) > 50:
            break
    if problems:
        raise NotTrainable(f"{split}: {len(problems)} item(s) not trainable:\n  " + "\n  ".join(problems[:20]))
    if n == 0:
        raise NotTrainable(f"{split}: manifest is empty")
    return {"split": split, "items": n, "manifest_sha256": hashlib.sha256(manifest.read_bytes()).hexdigest()}


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    corpus = Path(sys.argv[1])
    ok = True
    for split in sys.argv[2:]:
        try:
            print(json.dumps(check_trainable(corpus, split)))
        except NotTrainable as e:
            ok = False
            print(f"NOT TRAINABLE: {e}", file=sys.stderr)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
