"""Scorer and corpus tests: the metric must be exactly right before any engine is judged by it."""
import hashlib
import random
import unittest

import numpy as np

from pwml import corpus
from pwml.metrics import OcrLine, edit_distance, normalize, score_page


def as_ocr(gt, line, text=None, box=None):
    """A ground-truth line as an OCR result in image coordinates (through the page transform)."""
    A = np.array(gt["transform"])
    x0, y0, x1, y1 = box or line["box"]
    pts = [tuple(A[:, :2] @ np.array(p) + A[:, 2]) for p in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]]
    return OcrLine(line["text"] if text is None else text, pts)


class Metrics(unittest.TestCase):
    def test_edit_distance(self):
        self.assertEqual(edit_distance("kitten", "sitting"), 3)
        self.assertEqual(edit_distance("", "abc"), 3)
        self.assertEqual(edit_distance(["a", "b"], ["a", "c", "b"]), 1)
        self.assertEqual(normalize("  a’b — c  "), "a'b - c")

    def test_perfect_ocr_scores_zero_on_every_condition(self):
        fonts = corpus.load_fonts()
        for i, cond in enumerate(corpus.CONDITIONS):
            seed = corpus.SPLITS["blind"] + i * len(corpus.LAYOUTS)  # layout prose, condition i
            _, gt = corpus.render_page(seed, fonts, "blind")
            self.assertEqual(gt["condition"], cond)
            s = score_page(gt, [as_ocr(gt, l) for l in gt["lines"]]).summary()
            self.assertEqual((s["cer"], s["wer"], s["line_recall"], s["fp_lines"]), (0, 0, 1, 0), cond)

    def test_split_rows_and_errors_are_scored(self):
        fonts = corpus.load_fonts()
        _, gt = corpus.render_page(corpus.SPLITS["blind"] + 2, fonts, "blind")  # invoice
        row = next(l for l in gt["lines"] if l["text"].startswith("Description"))
        x0, y0, x1, y1 = row["box"]
        # A row returned as two pieces (cells) is reassembled left to right.
        mid = (x0 + x1) / 2
        rest = row["text"].split(" ", 1)
        pieces = [as_ocr(gt, row, rest[1], [mid, y0, x1, y1]), as_ocr(gt, row, rest[0], [x0, y0, mid - 5, y1])]
        others = [as_ocr(gt, l) for l in gt["lines"] if l is not row]
        self.assertEqual(score_page(gt, pieces + others).summary()["cer"], 0)
        # Hallucinated text far from any line counts as errors.
        ghost = OcrLine("GHOST TEXT", [(5, 5), (60, 5), (60, 20), (5, 20)])
        s = score_page(gt, others + [as_ocr(gt, row), ghost]).summary()
        self.assertEqual(s["fp_lines"], 1)
        self.assertGreater(s["cer"], 0)
        # A wrong character is one error.
        bad = as_ocr(gt, row, row["text"].replace("D", "O", 1))
        self.assertEqual(score_page(gt, others + [bad]).char_errors, 1)

    def test_axis_aligned_boxes_on_a_skewed_page_still_match(self):
        fonts = corpus.load_fonts()
        seed = corpus.SPLITS["blind"] + corpus.CONDITIONS.index("skew") * len(corpus.LAYOUTS)
        _, gt = corpus.render_page(seed, fonts, "blind")
        self.assertGreater(abs(gt["params"]["rotate_deg"]), 0.5)
        ocr = []
        for l in gt["lines"]:
            poly = as_ocr(gt, l).poly
            xs, ys = [p[0] for p in poly], [p[1] for p in poly]
            # What engines like Tesseract report: the axis-aligned box of the rotated line.
            ocr.append(OcrLine(l["text"], [(min(xs), min(ys)), (max(xs), min(ys)), (max(xs), max(ys)), (min(xs), max(ys))]))
        s = score_page(gt, ocr).summary()
        self.assertEqual((s["cer"], s["fp_lines"]), (0, 0))

    def test_merging_two_columns_is_not_free(self):
        fonts = corpus.load_fonts()
        _, gt = corpus.render_page(corpus.SPLITS["blind"] + 1, fonts, "blind")  # two_column
        self.assertEqual(gt["layout"], "two_column")
        ys = {}
        for l in gt["lines"][1:]:
            ys.setdefault(l["box"][1], []).append(l)
        pair = next(v for v in ys.values() if len(v) == 2)
        a, b = sorted(pair, key=lambda l: l["box"][0])
        merged = as_ocr(gt, a, a["text"] + " " + b["text"], [a["box"][0], a["box"][1], b["box"][2], b["box"][3]])
        others = [as_ocr(gt, l) for l in gt["lines"] if l is not a and l is not b]
        self.assertGreater(score_page(gt, others + [merged]).summary()["cer"], 0)


class Corpus(unittest.TestCase):
    def test_deterministic(self):
        fonts = corpus.load_fonts()
        h = lambda: hashlib.sha256(corpus.render_page(corpus.SPLITS["train"] + 5, fonts, "train")[0].image.tobytes()).hexdigest()
        self.assertEqual(h(), h())

    def test_heldout_fonts_never_in_train_or_val(self):
        fonts = corpus.load_fonts()
        held = {f.id for f in fonts if f.role == "heldout"}
        for split in ("train", "val"):
            for i in range(40):
                _, gt = corpus.render_page(corpus.SPLITS[split] + i, fonts, split)
                self.assertFalse(held & set(gt["fonts"]), (split, i))
        blind = [corpus.render_page(corpus.SPLITS["blind"] + i, fonts, "blind")[1] for i in range(40)]
        self.assertTrue(any(g["heldout_font"] for g in blind))

    def test_rotation_transform_follows_the_ink(self):
        img = corpus.Image.new("L", (400, 300), 255)
        img.putpixel((300, 60), 0)
        for deg in (2.5, -3.0):
            r = corpus.degrade(img, "skew", random.Random(1))
            d = r.params["rotate_deg"]
            A = np.array(r.transform)
            x, y = A[:, :2] @ np.array([300.0, 60.0]) + A[:, 2]
            a = np.asarray(r.image)
            ys, xs = np.nonzero(a < 200)
            self.assertLess(abs(xs.mean() - x) + abs(ys.mean() - y), 2.0, d)


if __name__ == "__main__":
    unittest.main()
