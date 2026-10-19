"""Deterministic synthetic OCR corpus: rendered document pages with line-level ground truth and
simulated scan conditions. Rights-clean by construction (PRD 15): authored text (textgen), fonts
whose licenses are recorded and hash-verified (manifests/fonts.json), no third-party documents.

Ground-truth boxes are in clean page coordinates; each page records the affine transform its
degradations applied (`transform`, clean → image), so evaluation can map OCR boxes back.

Usage: python -m pwml.corpus OUT_DIR [--train N] [--val N] [--blind N]
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import math
import random
import subprocess
from dataclasses import dataclass
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

from . import textgen

GENERATOR_VERSION = "1"
DPI = 200
PAGE_W, PAGE_H = int(8.5 * DPI), int(11 * DPI)
MARGIN = int(0.8 * DPI)
LAYOUTS = ["prose", "two_column", "invoice", "mixed"]
CONDITIONS = ["clean", "lowres", "blur", "noise", "jpeg", "skew", "faded", "shadow", "badscan"]
SPLITS = {"train": 1_000_000, "val": 2_000_000, "blind": 3_000_000}
MANIFESTS = Path(__file__).resolve().parent.parent / "manifests"


def pt(points: float) -> int:
    return round(points * DPI / 72)


@dataclass
class Font:
    id: str
    path: str
    role: str
    sha256: str


def load_fonts(manifest: Path = MANIFESTS / "fonts.json", font_dir: Path | None = None) -> list[Font]:
    """Fonts from the manifest, each verified by SHA-256. `font_dir` overrides the system path
    (e.g. a Kaggle dataset holding the same files)."""
    data = json.loads(manifest.read_text())
    fonts = []
    for f in data["fonts"]:
        path = str(font_dir / f["file"]) if font_dir else str(MANIFESTS.parent / f["path"])
        digest = hashlib.sha256(Path(path).read_bytes()).hexdigest()
        if digest != f["sha256"]:
            raise SystemExit(f"font {f['id']} at {path} does not match its recorded SHA-256; refusing to render")
        fonts.append(Font(f["id"], path, f["role"], digest))
    return fonts


class Page:
    def __init__(self) -> None:
        self.img = Image.new("L", (PAGE_W, PAGE_H), 255)
        self.draw = ImageDraw.Draw(self.img)
        self.lines: list[dict] = []
        self._cache: dict[tuple[str, int], ImageFont.FreeTypeFont] = {}

    def font(self, f: Font, size: int) -> ImageFont.FreeTypeFont:
        key = (f.path, size)
        if key not in self._cache:
            self._cache[key] = ImageFont.truetype(f.path, size)
        return self._cache[key]

    def text(self, x: int, y: int, s: str, f: Font, size: int, ink: int = 0) -> tuple[int, int, int, int]:
        """Draw `s` with its top-left at (x, y) and record it as one ground-truth line."""
        ft = self.font(f, size)
        self.draw.text((x, y), s, font=ft, fill=ink)
        l, t, r, b = self.draw.textbbox((x, y), s, font=ft)
        self.lines.append({"text": s, "box": [l, t, r, b], "font": f.id, "size_px": size})
        return l, t, r, b

    def row(self, x: int, y: int, cells: list[tuple[int, str]], f: Font, size: int) -> int:
        """Table row: cells at column offsets, recorded as ONE line (cells joined by spaces), so an
        engine may return the row whole or cell by cell."""
        ft = self.font(f, size)
        boxes = []
        for dx, s in cells:
            if s:
                self.draw.text((x + dx, y), s, font=ft, fill=0)
                boxes.append(self.draw.textbbox((x + dx, y), s, font=ft))
        if boxes:
            box = [min(b[0] for b in boxes), min(b[1] for b in boxes), max(b[2] for b in boxes), max(b[3] for b in boxes)]
            self.lines.append({"text": " ".join(s for _, s in cells if s), "box": box, "font": f.id, "size_px": size})
        return y + round(size * 1.45)


def wrap(text: str, ft: ImageFont.FreeTypeFont, width: int) -> list[str]:
    out, cur = [], ""
    for w in text.split():
        cand = f"{cur} {w}" if cur else w
        if cur and ft.getlength(cand) > width:
            out.append(cur)
            cur = w
        else:
            cur = cand
    if cur:
        out.append(cur)
    return out


def layout_prose(p: Page, r: random.Random, body: Font, head: Font, columns: int) -> None:
    y = MARGIN
    l, t, rr, b = p.text(MARGIN, y, " ".join(textgen.word(r) for _ in range(r.randint(3, 7))).title(), head, pt(r.choice([16, 18, 20])))
    y = b + pt(14)
    size = pt(r.choice([9, 10, 10.5, 11, 12]))
    lead = round(size * r.uniform(1.3, 1.55))
    gutter = pt(18)
    col_w = (PAGE_W - 2 * MARGIN - gutter * (columns - 1)) // columns
    ft = p.font(body, size)
    for c in range(columns):
        x = MARGIN + c * (col_w + gutter)
        cy = y
        while cy < PAGE_H - MARGIN - lead * 3:
            para = textgen.paragraph(r, r.randint(2, 5))
            for line in wrap(para, ft, col_w):
                if cy > PAGE_H - MARGIN - lead:
                    break
                p.text(x, cy, line, body, size)
                cy += lead
            cy += round(lead * 0.6)


def layout_invoice(p: Page, r: random.Random, body: Font, head: Font) -> None:
    y = MARGIN
    _, _, _, b = p.text(MARGIN, y, r.choice(["INVOICE", "Invoice", "Statement", "Purchase Order", "Receipt"]), head, pt(r.choice([20, 24])))
    y = b + pt(16)
    size = pt(r.choice([9, 10, 11]))
    fields = r.sample(textgen.FIELDS, k=r.randint(5, len(textgen.FIELDS)))
    for label, gen in fields:
        y = p.row(MARGIN, y, [(0, f"{label}:"), (pt(130), gen(r))], body, size)
    y += pt(16)
    cols = [0, pt(250), pt(320), pt(420)]
    y = p.row(MARGIN, y, list(zip(cols, ["Description", "Qty", "Unit price", "Total"])), head, size)
    for _ in range(r.randint(4, 14)):
        if y > PAGE_H - MARGIN - pt(80):
            break
        y = p.row(MARGIN, y, list(zip(cols, textgen.item_row(r))), body, size)
    y += pt(10)
    for label in ["Subtotal", "Tax", "Total due"]:
        y = p.row(MARGIN, y, [(pt(320), label), (pt(420), textgen.money(r))], body, size)
    p.text(MARGIN, PAGE_H - MARGIN, f"Questions? {textgen.email(r)} · {textgen.phone(r)}", body, pt(8))


def layout_mixed(p: Page, r: random.Random, body: Font, head: Font) -> None:
    y = MARGIN
    width = PAGE_W - 2 * MARGIN
    while y < PAGE_H - MARGIN - pt(120):
        _, _, _, b = p.text(MARGIN, y, f"{r.randint(1, 12)}. " + " ".join(textgen.word(r) for _ in range(r.randint(2, 5))).capitalize(), head, pt(r.choice([12, 14])))
        y = b + pt(8)
        size = pt(r.choice([8, 9, 10, 11]))
        ft = p.font(body, size)
        for line in wrap(textgen.paragraph(r, r.randint(1, 3)), ft, width):
            p.text(MARGIN, y, line, body, size)
            y += round(size * 1.4)
        for _ in range(r.randint(0, 4)):
            p.text(MARGIN + pt(14), y, "• " + textgen.sentence(r, 3, 9), body, size)
            y += round(size * 1.4)
        y += pt(12)


@dataclass
class Rendered:
    image: Image.Image
    transform: list[list[float]]  # 2x3 affine: clean page coords → image coords
    params: dict


def degrade(img: Image.Image, cond: str, r: random.Random) -> Rendered:
    """Apply a scan condition. Geometry changes are composed into one affine transform."""
    A = np.array([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]])
    params: dict = {"condition": cond}

    def rotate(im: Image.Image, deg: float) -> Image.Image:
        nonlocal A
        cx, cy = im.width / 2, im.height / 2
        th = math.radians(deg)
        # PIL rotates counter-clockwise on screen (y down): (x, y) → centre + (dx·cos + dy·sin, −dx·sin + dy·cos).
        R = np.array([[math.cos(th), math.sin(th)], [-math.sin(th), math.cos(th)]])
        t = np.array([cx, cy]) - R @ np.array([cx, cy])
        A = np.hstack([R @ A[:, :2], (R @ A[:, 2] + t)[:, None]])
        return im.rotate(deg, resample=Image.BICUBIC, expand=False, fillcolor=255)

    def scale(im: Image.Image, k: float) -> Image.Image:
        nonlocal A
        A = A * k
        return im.resize((round(im.width * k), round(im.height * k)), Image.LANCZOS)

    def noise(im: Image.Image, sigma: float, speckle: float) -> Image.Image:
        a = np.asarray(im, dtype=np.float32)
        rng = np.random.default_rng(r.randrange(2**32))
        a = a + rng.normal(0, sigma, a.shape)
        mask = rng.random(a.shape) < speckle
        a[mask] = rng.choice([0, 255], size=int(mask.sum()))
        return Image.fromarray(np.clip(a, 0, 255).astype(np.uint8))

    def jpeg(im: Image.Image, q: int) -> Image.Image:
        buf = io.BytesIO()
        im.save(buf, "JPEG", quality=q)
        return Image.open(io.BytesIO(buf.getvalue())).convert("L")

    def faded(im: Image.Image, contrast: float, lift: int) -> Image.Image:
        a = np.asarray(im, dtype=np.float32)
        a = 255 - (255 - a) * contrast
        return Image.fromarray(np.clip(a + lift - lift * a / 255, 0, 255).astype(np.uint8))

    def shadow(im: Image.Image, depth: float) -> Image.Image:
        a = np.asarray(im, dtype=np.float32)
        h, w = a.shape
        gx = np.linspace(0, 1, w)[None, :] ** r.uniform(1.5, 3)
        if r.random() < 0.5:
            gx = gx[:, ::-1]
        a = a * (1 - depth * gx)
        return Image.fromarray(np.clip(a, 0, 255).astype(np.uint8))

    im = img
    if cond == "lowres":
        k = r.choice([0.5, 0.6, 0.75])
        params["scale"] = k
        im = scale(im, k)
    elif cond == "blur":
        rad = round(r.uniform(1.0, 2.0), 2)
        params["blur_radius"] = rad
        im = im.filter(ImageFilter.GaussianBlur(rad))
    elif cond == "noise":
        s, sp = round(r.uniform(10, 25), 1), round(r.uniform(0.0005, 0.003), 4)
        params.update(sigma=s, speckle=sp)
        im = noise(im, s, sp)
    elif cond == "jpeg":
        q = r.randint(10, 30)
        params["jpeg_quality"] = q
        im = jpeg(im, q)
    elif cond == "skew":
        d = round(r.choice([-1, 1]) * r.uniform(0.7, 3.0), 2)
        params["rotate_deg"] = d
        im = rotate(im, d)
    elif cond == "faded":
        c, lift = round(r.uniform(0.35, 0.6), 2), r.randint(20, 60)
        params.update(contrast=c, lift=lift)
        im = faded(im, c, lift)
    elif cond == "shadow":
        dp = round(r.uniform(0.3, 0.6), 2)
        params["shadow_depth"] = dp
        im = shadow(im, dp)
    elif cond == "badscan":
        d = round(r.choice([-1, 1]) * r.uniform(0.5, 2.0), 2)
        k = r.choice([0.6, 0.75])
        rad = round(r.uniform(0.6, 1.2), 2)
        s = round(r.uniform(6, 14), 1)
        q = r.randint(25, 45)
        c = round(r.uniform(0.55, 0.8), 2)
        params.update(rotate_deg=d, scale=k, blur_radius=rad, sigma=s, jpeg_quality=q, contrast=c)
        im = rotate(im, d)
        im = im.filter(ImageFilter.GaussianBlur(rad))
        im = scale(im, k)
        im = faded(im, c, 25)
        im = noise(im, s, 0.0008)
        im = jpeg(im, q)
    return Rendered(im, A.tolist(), params)


def render_page(seed: int, fonts: list[Font], split: str) -> tuple[Rendered, dict]:
    r = random.Random(seed)
    if split != "blind":
        allowed = [f for f in fonts if f.role == "train"]
    elif r.random() < 0.4:
        # Two in five blind pages use only typefaces never seen in training.
        allowed = [f for f in fonts if f.role == "heldout"]
    else:
        allowed = fonts
    body, head = r.choice(allowed), r.choice(allowed)
    # Every run of len(LAYOUTS) × len(CONDITIONS) consecutive pages covers each combination once.
    idx = seed - SPLITS[split]
    layout = LAYOUTS[idx % len(LAYOUTS)]
    cond = CONDITIONS[(idx // len(LAYOUTS)) % len(CONDITIONS)]
    p = Page()
    if layout == "prose":
        layout_prose(p, r, body, head, 1)
    elif layout == "two_column":
        layout_prose(p, r, body, head, 2)
    elif layout == "invoice":
        layout_invoice(p, r, body, head)
    else:
        layout_mixed(p, r, body, head)
    rendered = degrade(p.img, cond, r)
    gt = {
        "id": f"{split}-{seed}",
        "split": split,
        "seed": seed,
        "layout": layout,
        "condition": cond,
        "params": rendered.params,
        "dpi": DPI,
        "page_size": [PAGE_W, PAGE_H],
        "image_size": list(rendered.image.size),
        "transform": rendered.transform,
        "fonts": sorted({body.id, head.id}),
        "heldout_font": any(f.role == "heldout" for f in fonts if f.id in (body.id, head.id)),
        "lines": p.lines,
    }
    return rendered, gt


def git_commit() -> str:
    try:
        return subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True, check=True, cwd=Path(__file__).parent).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def generate(out: Path, counts: dict[str, int], font_dir: Path | None = None) -> dict:
    fonts = load_fonts(font_dir=font_dir)
    out.mkdir(parents=True, exist_ok=True)
    commit = git_commit()
    summary = {"generator_version": GENERATOR_VERSION, "code_commit": commit, "splits": {}}
    for split, n in counts.items():
        d = out / split
        d.mkdir(exist_ok=True)
        rows = []
        n_lines = 0
        for i in range(n):
            seed = SPLITS[split] + i
            rendered, gt = render_page(seed, fonts, split)
            img_path = d / f"{gt['id']}.png"
            rendered.image.save(img_path, optimize=False)
            n_lines += len(gt["lines"])
            (d / f"{gt['id']}.json").write_text(json.dumps(gt, ensure_ascii=False))
            rows.append({
                # PRD 15 provenance fields.
                "id": gt["id"],
                "image": f"{split}/{img_path.name}",
                "image_sha256": hashlib.sha256(img_path.read_bytes()).hexdigest(),
                "source": "synthetic:pwml.corpus",
                "generator_version": GENERATOR_VERSION,
                "code_commit": commit,
                "seed": seed,
                "fonts": gt["fonts"],
                "rights": {
                    "status": "approved",
                    "basis": "Generated by the PageWeave project from authored text and recorded-license fonts (manifests/fonts.json); no third-party documents.",
                    "training": True,
                    "weight_redistribution": True,
                    "commercial": True,
                },
                "pii": "synthetic only (example.com addresses, 555 phone numbers, fictitious names)",
            })
        manifest = d / "manifest.jsonl"
        manifest.write_text("".join(json.dumps(x, ensure_ascii=False) + "\n" for x in rows))
        summary["splits"][split] = {
            "pages": n,
            "lines": n_lines,
            "manifest_sha256": hashlib.sha256(manifest.read_bytes()).hexdigest(),
        }
        if split == "blind":
            # Sealed: training tools refuse to read a split with this marker (pwml.rights.check_trainable).
            (d / "SEALED").write_text("Blind test split. Never use for training or tuning.\n")
    (out / "corpus.json").write_text(json.dumps(summary, indent=2) + "\n")
    return summary


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("out", type=Path)
    ap.add_argument("--train", type=int, default=0)
    ap.add_argument("--val", type=int, default=0)
    ap.add_argument("--blind", type=int, default=36)
    ap.add_argument("--font-dir", type=Path)
    a = ap.parse_args()
    counts = {k: v for k, v in (("train", a.train), ("val", a.val), ("blind", a.blind)) if v > 0}
    print(json.dumps(generate(a.out, counts, a.font_dir), indent=2))


if __name__ == "__main__":
    main()
