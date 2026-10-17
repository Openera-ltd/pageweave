"""Line-crop datasets for recognizer fine-tuning, cut the way the product cuts them.

Each page is run through the same PP-OCRv6 detection as inference; a detected box becomes a training
pair only when it matches exactly one ground-truth line (by centre, as the scorer does) and spans at
least 90% of that line's width, so the label is exactly the text inside the crop. Crops use
RapidOCR's `get_rotate_crop_image` (perspective warp, bicubic) — the cropping the Rust pipeline
reproduces. Rights are inherited from the page manifest; only rights-approved pages are used, and a
sealed split is refused.

Usage: python -m pwml.lines CORPUS_DIR SPLIT OUT_DIR [--limit N]
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image

from .metrics import normalize, to_page
from .rights import check_trainable

MIN_COVER = 0.9


def match_boxes(gt: dict, polys: list[np.ndarray]) -> list[tuple[int, int]]:
    """(detected index, gt line index) pairs that match one-to-one and cover the whole line."""
    boxes = np.array([l["box"] for l in gt["lines"]], dtype=float)
    hits: dict[int, list[int]] = {}
    for di, poly in enumerate(polys):
        p = to_page(gt["transform"], poly)
        cx, cy = p.mean(axis=0)
        inside = np.nonzero((boxes[:, 0] <= cx) & (cx <= boxes[:, 2]) & (boxes[:, 1] <= cy) & (cy <= boxes[:, 3]))[0]
        if len(inside) != 1:
            continue
        li = int(inside[0])
        x0, x1 = p[:, 0].min(), p[:, 0].max()
        width = boxes[li, 2] - boxes[li, 0]
        covered = max(0.0, min(x1, boxes[li, 2]) - max(x0, boxes[li, 0]))
        if width > 0 and covered / width >= MIN_COVER:
            hits.setdefault(li, []).append(di)
    return [(ds[0], li) for li, ds in hits.items() if len(ds) == 1]


def build(corpus: Path, split: str, out: Path, limit: int | None = None) -> dict:
    from rapidocr import RapidOCR
    from rapidocr.utils.process_img import get_rotate_crop_image

    sdir = corpus / split
    gate = check_trainable(corpus, split)  # refuses sealed splits and items without approved rights
    eng = RapidOCR(params={"Global.use_cls": False, "Global.log_level": "error"})
    (out / "crops").mkdir(parents=True, exist_ok=True)
    labels = []
    pages = skipped_rights = lines_total = 0
    manifest = [json.loads(l) for l in (sdir / "manifest.jsonl").read_text(encoding="utf-8").splitlines() if l.strip()]
    for entry in manifest[: limit or None]:
        rights = entry.get("rights", {})
        if rights.get("status") != "approved" or not rights.get("training"):
            skipped_rights += 1
            continue
        gt = json.loads((sdir / f"{entry['id']}.json").read_text(encoding="utf-8"))
        img = np.array(Image.open(corpus / entry["image"]).convert("RGB"))[:, :, ::-1].copy()  # BGR, like inference
        det = eng(img, use_det=True, use_cls=False, use_rec=False)
        polys = [np.asarray(b, dtype=np.float32) for b in (det.boxes if det.boxes is not None else [])]
        pages += 1
        lines_total += len(gt["lines"])
        for di, li in match_boxes(gt, polys):
            text = normalize(gt["lines"][li]["text"])
            if not text:
                continue
            crop = get_rotate_crop_image(img, polys[di])
            h, w = crop.shape[:2]
            if h * 1.0 / max(1, w) >= 1.5:
                crop = np.rot90(crop)
            name = f"{entry['id']}-{li:03d}.png"
            Image.fromarray(crop[:, :, ::-1]).save(out / "crops" / name, optimize=True)
            labels.append({"image": f"crops/{name}", "text": text, "page": entry["id"], "condition": gt["condition"], "font": gt["lines"][li].get("font")})
    with (out / "labels.jsonl").open("w", encoding="utf-8") as f:
        for l in labels:
            f.write(json.dumps(l, ensure_ascii=False) + "\n")
    summary = {
        "corpus": str(corpus.name),
        "split": split,
        "pages": pages,
        "skipped_not_trainable": skipped_rights,
        "gt_lines": lines_total,
        "pairs": len(labels),
        "coverage": round(len(labels) / max(1, lines_total), 4),
        "min_cover": MIN_COVER,
        "labels_sha256": hashlib.sha256((out / "labels.jsonl").read_bytes()).hexdigest(),
        "manifest_sha256": gate["manifest_sha256"],
        "rights": "inherited from page manifests (synthetic, approved for training and weight redistribution)",
    }
    (out / "lines.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    return summary


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("corpus", type=Path)
    ap.add_argument("split")
    ap.add_argument("out", type=Path)
    ap.add_argument("--limit", type=int)
    a = ap.parse_args()
    print(json.dumps(build(a.corpus, a.split, a.out, a.limit), indent=2))


if __name__ == "__main__":
    main()
