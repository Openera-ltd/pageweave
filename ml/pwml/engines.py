"""Baseline OCR engines behind one interface: `name`, `info()` and `run(image_path) -> [OcrLine]`."""
from __future__ import annotations

import csv
import io
import subprocess
from pathlib import Path

from .metrics import OcrLine


class Tesseract:
    """Tesseract 5 (Apache-2.0) via its CLI, line level from TSV output (words grouped by line)."""

    def __init__(self, lang: str = "eng", psm: int = 3, oem: int = 1) -> None:
        self.lang, self.psm, self.oem = lang, psm, oem
        self.name = f"tesseract-{lang}-psm{psm}"

    def info(self) -> dict:
        v = subprocess.run(["tesseract", "--version"], capture_output=True, text=True).stdout.splitlines()[0]
        return {"engine": "tesseract", "version": v, "lang": self.lang, "psm": self.psm, "oem": self.oem, "license": "Apache-2.0"}

    def run(self, image: Path) -> list[OcrLine]:
        out = subprocess.run(
            ["tesseract", str(image), "-", "--oem", str(self.oem), "--psm", str(self.psm), "-l", self.lang, "tsv"],
            capture_output=True, text=True, check=True,
        ).stdout
        groups: dict[tuple, list[dict]] = {}
        for row in csv.DictReader(io.StringIO(out), delimiter="\t", quoting=csv.QUOTE_NONE):
            if row.get("level") != "5" or not (row.get("text") or "").strip():
                continue
            key = (row["page_num"], row["block_num"], row["par_num"], row["line_num"])
            groups.setdefault(key, []).append(row)
        lines = []
        for words in groups.values():
            x0 = min(int(w["left"]) for w in words)
            y0 = min(int(w["top"]) for w in words)
            x1 = max(int(w["left"]) + int(w["width"]) for w in words)
            y1 = max(int(w["top"]) + int(w["height"]) for w in words)
            text = " ".join(w["text"] for w in sorted(words, key=lambda w: int(w["left"])))
            conf = sum(float(w["conf"]) for w in words) / len(words) / 100
            lines.append(OcrLine(text, [(x0, y0), (x1, y0), (x1, y1), (x0, y1)], conf))
        return lines


def normalize_background(path: Path):
    """Flatten uneven illumination and stretch contrast: divide by a large-blur estimate of the paper
    background, then map the darkest 0.5 % to black and the paper to white. Returns an RGB array."""
    import numpy as np
    from PIL import Image, ImageFilter

    im = Image.open(path).convert("L")
    small = im.resize((max(1, im.width // 8), max(1, im.height // 8)), Image.BILINEAR)
    # Max filter removes text strokes from the estimate, blur smooths it.
    bg = small.filter(ImageFilter.MaxFilter(7)).filter(ImageFilter.GaussianBlur(6)).resize(im.size, Image.BILINEAR)
    a = np.asarray(im, dtype=np.float32) / np.maximum(np.asarray(bg, dtype=np.float32), 1.0)
    lo = float(np.percentile(a, 0.5))
    a = np.clip((a - lo) / max(1e-3, 1.0 - lo), 0, 1) * 255
    g = a.astype(np.uint8)
    return np.stack([g, g, g], axis=-1)


class PpOcr:
    """PP-OCRv6 small detection + recognition (Apache-2.0 weights) on ONNX Runtime, via RapidOCR."""

    def __init__(self, use_cls: bool = False, threads: int | None = None, normalize: bool = False, det: dict | None = None, extra: dict | None = None, label: str = "") -> None:
        self.normalize = normalize
        self.det = det or {}
        self.extra = extra or {}
        from rapidocr import RapidOCR

        params = {"Global.use_cls": use_cls, "Global.log_level": "error"}
        if threads:
            params["EngineConfig.onnxruntime.intra_op_num_threads"] = threads
        params.update({f"Det.{k}": v for k, v in self.det.items()})
        params.update(self.extra)
        self._eng = RapidOCR(params=params)
        self.use_cls = use_cls
        self.name = "ppocrv6-small" + ("-cls" if use_cls else "") + ("-norm" if normalize else "") + "".join(f"-{k}{v}" for k, v in self.det.items()) + (f"-{label}" if label else "")

    def info(self) -> dict:
        import onnxruntime
        import rapidocr

        models = Path(rapidocr.__file__).parent / "models"
        import hashlib

        return {
            "engine": "PP-OCRv6 small (det+rec) via RapidOCR",
            "rapidocr": rapidocr.__version__ if hasattr(rapidocr, "__version__") else "3.10.0",
            "onnxruntime": onnxruntime.__version__,
            "models": {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(models.glob("*.onnx"))},
            "license": "Apache-2.0 (code and weights)",
            "use_cls": self.use_cls,
            "normalize_background": self.normalize,
            "det_overrides": self.det,
            "overrides": {k: (Path(v).name if isinstance(v, str) and "/" in v else v) for k, v in self.extra.items()},
        }

    def run(self, image: Path) -> list[OcrLine]:
        r = self._eng(normalize_background(image) if self.normalize else str(image))
        if r.boxes is None:
            return []
        return [OcrLine(t, [tuple(map(float, p)) for p in box], float(s)) for box, t, s in zip(r.boxes, r.txts, r.scores)]


OFFICIAL = Path(__import__("os").environ.get("PWML_OFFICIAL_MODELS", "/nonexistent"))


def _official(paddle_cfg: bool) -> "PpOcr":
    extra = {
        "Det.model_path": str(OFFICIAL / "det_inference.onnx"),
        "Rec.model_path": str(OFFICIAL / "rec_inference.onnx"),
        "Rec.rec_keys_path": str(OFFICIAL / "rec_dict.txt"),
    }
    if paddle_cfg:
        # Settings from the official model's inference.yml.
        extra.update({"Det.mean": [0.485, 0.456, 0.406], "Det.std": [0.229, 0.224, 0.225], "Det.thresh": 0.2, "Det.box_thresh": 0.45, "Det.unclip_ratio": 1.4})
    return PpOcr(extra=extra, label="official" + ("-paddlecfg" if paddle_cfg else "-rapidcfg"))


class External:
    """Results produced outside Python (e.g. the Rust engine via crates/pw-ocr/examples/ocr_dir.rs)
    and scored with `bench --rescore`; cannot be run from here."""

    def __init__(self) -> None:
        raise SystemExit("this engine's raw output is produced by its own harness; use bench --rescore")


ENGINES = {
    "pwocr": External,
    "ppocr-official-rapidcfg": lambda: _official(False),
    "ppocr-official-paddlecfg": lambda: _official(True),
    "tesseract": Tesseract,
    "ppocr": PpOcr,
    "ppocr-norm": lambda: PpOcr(normalize=True),
    "ppocr-nodil": lambda: PpOcr(det={"use_dilation": False}),
    "ppocr-unclip14": lambda: PpOcr(det={"unclip_ratio": 1.4}),
    "ppocr-nodil-unclip14": lambda: PpOcr(det={"use_dilation": False, "unclip_ratio": 1.4}),
}
