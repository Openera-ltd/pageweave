"""Fine-tune the PP-OCRv6 small recognizer on rights-clean synthetic line crops (PRD 14).

Self-contained Kaggle script. Inputs (one private dataset): `lines-train/` and `lines-val/` (crops +
labels.jsonl from `pwml.lines`), the shipped `rec.onnx` + `rec_dict.txt` (Apache-2.0) and an
onnx2torch wheel (no internet on the run). The ONNX graph is converted to PyTorch only to compute
gradients; trained values are written back into the ORIGINAL graph's initializers, so the exported
model keeps the shipped graph (and its dynamic width) exactly. Model selection uses the validation
split only; the sealed blind split never reaches this run.

Outputs in /kaggle/working: rec-finetuned.onnx (best validation CER), metrics.json, SHA256SUMS. They are
checkpointed after the baseline and after every epoch, and a wall-clock watchdog ends the whole
process at PW_HARD_DEADLINE_S (default 2.75 h, under the 3 h run ceiling) wherever it is — loading,
training, evaluating or exporting — leaving the last checkpoint with metrics.json "status":
"stopped_at_deadline". (2026-10-17: a run without this watchdog kept its GPU session for 12 h.)
Local smoke test: PW_INPUT=dir PW_OUT=dir PW_MAX_STEPS=3 python train.py
"""
import glob
import hashlib
import json
import math
import os
import random
import subprocess
import sys
import threading
import time
from pathlib import Path

INPUT = Path(os.environ.get("PW_INPUT", "/kaggle/input"))
OUT = Path(os.environ.get("PW_OUT", "/kaggle/working"))
MAX_STEPS = int(os.environ.get("PW_MAX_STEPS", "0"))  # 0 = full run
EPOCHS = int(os.environ.get("PW_EPOCHS", "4"))
LR = float(os.environ.get("PW_LR", "5e-5"))
BATCH = int(os.environ.get("PW_BATCH", "48"))
TIME_BUDGET_S = float(os.environ.get("PW_TIME_BUDGET_S", str(2.4 * 3600)))  # under the 3 h run ceiling
HARD_DEADLINE_S = float(os.environ.get("PW_HARD_DEADLINE_S", str(2.75 * 3600)))
SEED = 1234
started = time.time()
OUT.mkdir(parents=True, exist_ok=True)
OUTPUT_FILES = ("rec-finetuned.onnx", "metrics.json")
metrics = {"status": "starting"}


def write_atomic(path, data):
    tmp = path.with_name(path.name + f".tmp{threading.get_ident()}")
    tmp.write_bytes(data)
    os.replace(tmp, path)


def checkpoint(status):
    """metrics.json and SHA256SUMS for the files that exist now (complete files only: every
    output is replaced atomically)."""
    metrics["status"] = status
    metrics["seconds"] = round(time.time() - started)
    write_atomic(OUT / "metrics.json", json.dumps(metrics, indent=2).encode())
    sums = "".join(
        f"{hashlib.sha256((OUT / f).read_bytes()).hexdigest()}  {f}\n" for f in OUTPUT_FILES if (OUT / f).exists()
    )
    write_atomic(OUT / "SHA256SUMS", sums.encode())


def on_deadline():
    print(f"hard deadline ({HARD_DEADLINE_S:.0f} s) reached: stopping with the last checkpoint", flush=True)
    try:
        checkpoint("stopped_at_deadline")
    finally:
        os._exit(0)  # exit 0 so the platform keeps the outputs; the status says what happened


watchdog = threading.Timer(HARD_DEADLINE_S, on_deadline)
watchdog.daemon = True
watchdog.start()


def find(pattern):
    hits = sorted(glob.glob(str(INPUT / "**" / pattern), recursive=True))
    if not hits:
        raise SystemExit(f"missing input: {pattern}")
    return Path(hits[0])


try:
    import onnx2torch  # noqa: F401
except ImportError:
    wheel = find("onnx2torch-*.whl")
    subprocess.run([sys.executable, "-m", "pip", "install", "--no-index", "--no-deps", "-q", str(wheel)], check=True)

import numpy as np
import onnx
import torch
from onnx import helper, numpy_helper
from onnx2torch import convert
from PIL import Image

random.seed(SEED)
np.random.seed(SEED)
torch.manual_seed(SEED)
dev = torch.device("cuda" if torch.cuda.is_available() else "cpu")


# ---- model --------------------------------------------------------------------------------
def explicit_same_upper(model):
    for n in model.graph.node:
        if n.op_type not in ("Conv", "MaxPool", "AveragePool"):
            continue
        attrs = {a.name: helper.get_attribute_value(a) for a in n.attribute}
        ap = attrs.get("auto_pad", b"NOTSET")
        ap = ap.decode() if isinstance(ap, bytes) else ap
        if ap != "SAME_UPPER":
            continue
        if any(s != 1 for s in attrs.get("strides", [1, 1])):
            raise SystemExit(f"{n.name}: SAME_UPPER with stride > 1 is input-size dependent")
        total = [(k - 1) * d for k, d in zip(attrs["kernel_shape"], attrs.get("dilations", [1, 1]))]
        begin = [t // 2 for t in total]
        keep = [a for a in n.attribute if a.name != "auto_pad"]
        del n.attribute[:]
        n.attribute.extend(keep)
        n.attribute.append(helper.make_attribute("pads", begin + [t - b for t, b in zip(total, begin)]))


def value_key(a):
    a = np.ascontiguousarray(a)
    return (a.dtype.str, a.shape, hashlib.sha1(a.tobytes()).hexdigest())


onnx_path = find("rec.onnx")
original = onnx.load(str(onnx_path))
inits = {i.name: numpy_helper.to_array(i) for i in original.graph.initializer if numpy_helper.to_array(i).dtype.kind == "f" and numpy_helper.to_array(i).size > 1}
by_value = {}
for name, arr in inits.items():
    by_value.setdefault(value_key(arr), []).append(name)
conv_src = onnx.load(str(onnx_path))
explicit_same_upper(conv_src)
model = convert(conv_src)

# Float constants that onnx2torch stored as buffers become trainable parameters (BN running stats stay).
for mod in model.modules():
    for bname, buf in list(mod._buffers.items()):
        if buf is None or not buf.dtype.is_floating_point or buf.numel() <= 1 or bname in ("running_mean", "running_var"):
            continue
        del mod._buffers[bname]
        mod.register_parameter(bname, torch.nn.Parameter(buf.clone()))
mapping = {}  # torch name -> onnx initializer
for tname, t in list(model.named_parameters()) + list(model.named_buffers()):
    a = t.detach().cpu().numpy()
    if a.dtype.kind != "f" or a.size <= 1:
        continue
    hits = by_value.get(value_key(a), [])
    if len(hits) == 1:
        mapping[tname] = hits[0]
if set(mapping.values()) != set(inits):
    raise SystemExit(f"weight mapping incomplete: {len(set(mapping.values()))} of {len(inits)} initializers")
for tname, p in model.named_parameters():
    p.requires_grad_(tname in mapping)
model.to(dev)


def set_train_mode():
    model.train()
    for m in model.modules():  # frozen normalisation statistics: the data are small and synthetic
        if isinstance(m, torch.nn.modules.batchnorm._BatchNorm):
            m.eval()


def export(path):
    out = onnx.load(str(onnx_path))
    state = dict(model.named_parameters())
    state.update(dict(model.named_buffers()))
    by_name = {i.name: i for i in out.graph.initializer}
    for tname, iname in mapping.items():
        arr = state[tname].detach().float().cpu().numpy().astype(numpy_helper.to_array(by_name[iname]).dtype)
        by_name[iname].CopyFrom(numpy_helper.from_array(arr, iname))
    tmp = path.with_name(path.name + ".tmp")
    onnx.save(out, str(tmp))
    os.replace(tmp, path)


# ---- data ---------------------------------------------------------------------------------
chars = find("rec_dict.txt").read_text(encoding="utf-8").split("\n")
chars = [c for c in chars if c != ""] + [" "]
index = {c: i + 1 for i, c in enumerate(chars)}  # 0 = CTC blank
assert len(chars) + 1 == 18710, len(chars)


def load_split(name):
    labels = find(f"{name}/labels.jsonl")
    root = labels.parent
    items, dropped = [], 0
    for line in labels.read_text(encoding="utf-8").splitlines():
        r = json.loads(line)
        ids = [index.get(c) for c in r["text"]]
        if not ids or None in ids:
            dropped += 1
            continue
        items.append((root / r["image"], ids, r["text"]))
    return items, dropped


train_items, train_dropped = load_split("lines-train")
val_items, val_dropped = load_split("lines-val")
MAX_W = 1600


def to_tensor(path):
    img = Image.open(path).convert("RGB")
    w, h = img.size
    nw = max(8, min(MAX_W, int(math.ceil(48 * w / h))))
    img = img.resize((nw, 48), Image.BICUBIC)
    a = np.asarray(img, dtype=np.float32)[:, :, ::-1] / 255.0  # BGR, as at inference
    a = (a - 0.5) / 0.5
    return torch.from_numpy(a.transpose(2, 0, 1).copy())


def batches(items, shuffle):
    order = list(range(len(items)))
    widths = {}
    for i in order:
        with Image.open(items[i][0]) as im:
            widths[i] = im.size[0] / im.size[1]
    order.sort(key=lambda i: widths[i])  # similar widths together: little padding
    chunks = [order[i:i + BATCH] for i in range(0, len(order), BATCH)]
    if shuffle:
        random.shuffle(chunks)
    for ch in chunks:
        ts = [to_tensor(items[i][0]) for i in ch]
        W = max(max(t.shape[2] for t in ts), 320)
        x = torch.zeros(len(ts), 3, 48, W)
        for k, t in enumerate(ts):
            x[k, :, :, : t.shape[2]] = t
        yield x, [items[i][1] for i in ch], [items[i][2] for i in ch]


def decode(probs):
    out = []
    for seq in probs.argmax(-1).cpu().numpy():
        s, prev = [], 0
        for c in seq:
            if c != prev and c != 0:
                s.append(chars[c - 1])
            prev = c
        out.append("".join(s).strip())
    return out


def edit(a, b):
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i] + [0] * len(b)
        for j, cb in enumerate(b, 1):
            cur[j] = min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (ca != cb))
        prev = cur
    return prev[-1]


def evaluate(limit=None):
    model.eval()
    errs = chars_total = exact = n = 0
    with torch.no_grad():
        for x, _, texts in batches(val_items[:limit] if limit else val_items, False):
            with torch.autocast(device_type=dev.type, dtype=torch.float16, enabled=dev.type == "cuda"):
                probs = model(x.to(dev))
            for hyp, ref in zip(decode(probs.float()), texts):
                e = edit(hyp, ref)
                errs += e
                chars_total += len(ref)
                exact += e == 0
                n += 1
    return {"cer": errs / max(1, chars_total), "line_exact": exact / max(1, n), "lines": n, "chars": chars_total}


# ---- training -----------------------------------------------------------------------------
metrics.update({
    "train_lines": len(train_items),
    "train_dropped_unknown_chars": train_dropped,
    "val_lines": len(val_items),
    "val_dropped_unknown_chars": val_dropped,
    "device": torch.cuda.get_device_name(0) if dev.type == "cuda" else "cpu",
    "torch": torch.__version__,
    "hyper": {"epochs": EPOCHS, "lr": LR, "batch": BATCH, "seed": SEED, "bn": "frozen", "max_width": MAX_W},
    "onnx_input_sha256": hashlib.sha256(onnx_path.read_bytes()).hexdigest(),
    "epochs": [],
})
limit = 64 if MAX_STEPS else None
base = evaluate(limit)
metrics["baseline_val"] = base
metrics["best_val_cer"] = base["cer"]
metrics["best_epoch"] = 0
metrics["improved"] = False
print("baseline", base, flush=True)
params = [p for p in model.parameters() if p.requires_grad]
opt = torch.optim.AdamW(params, lr=LR, weight_decay=1e-4)
steps_per_epoch = math.ceil(len(train_items) / BATCH)
total = MAX_STEPS or EPOCHS * steps_per_epoch
sched = torch.optim.lr_scheduler.LambdaLR(opt, lambda s: min(1.0, (s + 1) / 100) * 0.5 * (1 + math.cos(math.pi * min(1.0, s / total))))
scaler = torch.amp.GradScaler(enabled=dev.type == "cuda")
best = base["cer"]
best_epoch = 0
export(OUT / "rec-finetuned.onnx")  # baseline weights until something beats them
checkpoint("training")
step = 0
stop = False
for epoch in range(1, EPOCHS + 1):
    set_train_mode()
    losses = []
    for x, targets, _ in batches(train_items, True):
        x = x.to(dev)
        with torch.autocast(device_type=dev.type, dtype=torch.float16, enabled=dev.type == "cuda"):
            probs = model(x)
        logp = torch.log(probs.float().clamp_min(1e-8)).permute(1, 0, 2)
        flat = torch.tensor([c for t in targets for c in t], dtype=torch.long)
        tl = torch.tensor([len(t) for t in targets], dtype=torch.long)
        il = torch.full((len(targets),), logp.shape[0], dtype=torch.long)
        loss = torch.nn.functional.ctc_loss(logp, flat, il, tl, blank=0, zero_infinity=True)
        opt.zero_grad(set_to_none=True)
        scaler.scale(loss).backward()
        scaler.unscale_(opt)
        torch.nn.utils.clip_grad_norm_(params, 5.0)
        scaler.step(opt)
        scaler.update()
        sched.step()
        losses.append(float(loss))
        step += 1
        if MAX_STEPS and step >= MAX_STEPS:
            stop = True
            break
        if time.time() - started > TIME_BUDGET_S:
            stop = True
            break
    v = evaluate(limit)
    metrics["epochs"].append({"epoch": epoch, "steps": step, "train_loss": float(np.mean(losses)) if losses else None, "val": v, "elapsed_s": round(time.time() - started)})
    print("epoch", epoch, metrics["epochs"][-1], flush=True)
    if v["cer"] < best:
        best, best_epoch = v["cer"], epoch
        export(OUT / "rec-finetuned.onnx")
    metrics.update(best_val_cer=best, best_epoch=best_epoch, improved=best_epoch > 0)
    checkpoint("training")
    if stop:
        break
watchdog.cancel()
metrics.update(best_val_cer=best, best_epoch=best_epoch, improved=best_epoch > 0)
checkpoint("complete")
print(json.dumps({k: metrics[k] for k in ("baseline_val", "best_val_cer", "best_epoch", "seconds")}, indent=2))
