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


class PpOcr:
    """PP-OCRv6 small detection + recognition (Apache-2.0 weights) on ONNX Runtime, via RapidOCR."""

    def __init__(self, use_cls: bool = False, threads: int | None = None) -> None:
        from rapidocr import RapidOCR

        params = {"Global.use_cls": use_cls, "Global.log_level": "error"}
        if threads:
            params["EngineConfig.onnxruntime.intra_op_num_threads"] = threads
        self._eng = RapidOCR(params=params)
        self.use_cls = use_cls
        self.name = "ppocrv6-small" + ("-cls" if use_cls else "")

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
        }

    def run(self, image: Path) -> list[OcrLine]:
        r = self._eng(str(image))
        if r.boxes is None:
            return []
        return [OcrLine(t, [tuple(map(float, p)) for p in box], float(s)) for box, t, s in zip(r.boxes, r.txts, r.scores)]


ENGINES = {"tesseract": Tesseract, "ppocr": PpOcr}
