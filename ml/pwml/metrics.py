"""OCR scoring against line-level ground truth.

Each OCR line (text + polygon in image coordinates) is mapped back into clean page coordinates with
the inverse of the page's degradation transform, then assigned to the ground-truth line containing
its centre (or, failing that, holding at least half its area). A ground-truth line's hypothesis is its assigned OCR
pieces left to right. Unassigned OCR text counts as insertions (false positives), so an engine
cannot score well by hallucinating text or by merging across columns.
"""
from __future__ import annotations

import unicodedata
from dataclasses import dataclass, field

import numpy as np


def normalize(s: str) -> str:
    """NFC, unify quotes/dashes/spaces that scanners legitimately confuse, collapse whitespace."""
    s = unicodedata.normalize("NFC", s)
    for a, b in (("‘", "'"), ("’", "'"), ("“", '"'), ("”", '"'), ("–", "-"), ("—", "-"), (" ", " ")):
        s = s.replace(a, b)
    return " ".join(s.split())


def edit_distance(a: str | list[str], b: str | list[str]) -> int:
    """Levenshtein distance (characters for strings, tokens for lists)."""
    if len(a) < len(b):
        a, b = b, a
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (ca != cb)))
        prev = cur
    return prev[-1]


@dataclass
class OcrLine:
    text: str
    poly: list[tuple[float, float]]  # image coordinates
    conf: float | None = None


@dataclass
class PageScore:
    chars: int = 0
    char_errors: int = 0
    words: int = 0
    word_errors: int = 0
    gt_lines: int = 0
    gt_lines_found: int = 0
    fp_lines: int = 0
    fp_chars: int = 0
    exact_lines: int = 0
    details: list[dict] = field(default_factory=list)

    def add(self, o: "PageScore") -> None:
        for k in ("chars", "char_errors", "words", "word_errors", "gt_lines", "gt_lines_found", "fp_lines", "fp_chars", "exact_lines"):
            setattr(self, k, getattr(self, k) + getattr(o, k))

    def summary(self) -> dict:
        return {
            "cer": round(self.char_errors / max(1, self.chars), 5),
            "wer": round(self.word_errors / max(1, self.words), 5),
            "line_recall": round(self.gt_lines_found / max(1, self.gt_lines), 5),
            "exact_line_rate": round(self.exact_lines / max(1, self.gt_lines), 5),
            "fp_lines": self.fp_lines,
            "chars": self.chars,
            "gt_lines": self.gt_lines,
        }


def to_page(transform: list[list[float]], pts: list[tuple[float, float]]) -> np.ndarray:
    A = np.array(transform, dtype=float)
    M, t = A[:, :2], A[:, 2]
    inv = np.linalg.inv(M)
    return (np.asarray(pts, dtype=float) - t) @ inv.T


def score_page(gt: dict, ocr: list[OcrLine], keep_details: bool = False) -> PageScore:
    lines = gt["lines"]
    boxes = np.array([l["box"] for l in lines], dtype=float)
    assigned: list[list[tuple[float, str]]] = [[] for _ in lines]
    s = PageScore()
    for o in ocr:
        text = normalize(o.text)
        if not text:
            continue
        p = to_page(gt["transform"], o.poly)
        x0, y0 = p.min(axis=0)
        x1, y1 = p.max(axis=0)
        area = max(1e-6, (x1 - x0) * (y1 - y0))
        iw = np.clip(np.minimum(boxes[:, 2], x1) - np.maximum(boxes[:, 0], x0), 0, None)
        ih = np.clip(np.minimum(boxes[:, 3], y1) - np.maximum(boxes[:, 1], y0), 0, None)
        frac = iw * ih / area
        # Primary rule: the line containing the OCR box's centre. Engines that report axis-aligned
        # boxes on a skewed page give boxes spanning several lines; their centre still identifies
        # the line. Fallback: the line holding at least half the box.
        cx, cy = p.mean(axis=0)
        inside = np.nonzero((boxes[:, 0] <= cx) & (cx <= boxes[:, 2]) & (boxes[:, 1] <= cy) & (cy <= boxes[:, 3]))[0]
        if len(inside):
            best = int(inside[np.argmax(frac[inside])])
        else:
            best = int(np.argmax(frac)) if len(lines) else -1
            if best >= 0 and frac[best] < 0.5:
                best = -1
        if best >= 0:
            assigned[best].append((x0, text))
        else:
            s.fp_lines += 1
            s.fp_chars += len(text)
    for line, parts in zip(lines, assigned):
        ref = normalize(line["text"])
        hyp = " ".join(t for _, t in sorted(parts))
        ce = edit_distance(ref, hyp)
        we = edit_distance(ref.split(), hyp.split())
        s.chars += len(ref)
        s.char_errors += ce
        s.words += len(ref.split())
        s.word_errors += we
        s.gt_lines += 1
        s.gt_lines_found += bool(parts)
        s.exact_lines += ce == 0
        if keep_details and ce:
            s.details.append({"ref": ref, "hyp": hyp, "cer": round(ce / max(1, len(ref)), 3), "font": line.get("font"), "size_px": line.get("size_px")})
    # Unassigned text is inserted text: count it as character and word errors.
    s.char_errors += s.fp_chars
    s.word_errors += s.fp_lines
    return s
