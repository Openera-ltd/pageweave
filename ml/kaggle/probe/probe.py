"""Kaggle environment probe (PRD 14): record the real hardware, libraries and limits before any
training run is designed around them, and check the attached corpus is intact. No secrets are read."""
import hashlib
import importlib
import json
import os
import platform
import shutil
import subprocess
import time
from pathlib import Path

OUT = Path("/kaggle/working")
report = {"started_unix": time.time(), "python": platform.python_version(), "platform": platform.platform()}
try:
    report["gpu"] = subprocess.run(["nvidia-smi", "--query-gpu=name,driver_version,memory.total", "--format=csv,noheader"], capture_output=True, text=True, timeout=30).stdout.strip()
except Exception as e:  # noqa: BLE001 - report, never fail the probe
    report["gpu"] = f"unavailable: {type(e).__name__}"
report["cpu_count"] = os.cpu_count()
try:
    report["ram_gb"] = round(os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES") / 1e9, 1)
except (ValueError, OSError):
    report["ram_gb"] = None
report["disk_working_free_gb"] = round(shutil.disk_usage(OUT).free / 1e9, 1)
libs = {}
for m in ("torch", "torchvision", "paddle", "onnx", "onnxruntime", "numpy", "PIL", "cv2"):
    try:
        mod = importlib.import_module(m)
        libs[m] = getattr(mod, "__version__", "?")
    except Exception:  # noqa: BLE001
        libs[m] = None
report["libraries"] = libs
try:
    import torch
    report["torch_cuda"] = torch.cuda.is_available()
    if torch.cuda.is_available():
        report["torch_device"] = torch.cuda.get_device_name(0)
except Exception:  # noqa: BLE001
    report["torch_cuda"] = None
# Attached corpus: verify every image against its manifest hash.
inputs = sorted(Path("/kaggle/input").glob("*/"))
report["inputs"] = [p.name for p in inputs]
checked, bad = 0, 0
for man in Path("/kaggle/input").rglob("manifest.jsonl"):
    root = man.parent.parent
    for line in man.read_text().splitlines():
        item = json.loads(line)
        f = root / item["image"]
        checked += 1
        if not f.exists() or hashlib.sha256(f.read_bytes()).hexdigest() != item["image_sha256"]:
            bad += 1
report["corpus_images_checked"] = checked
report["corpus_images_bad"] = bad
report["seconds"] = round(time.time() - report["started_unix"], 1)
(OUT / "probe.json").write_text(json.dumps(report, indent=2))
(OUT / "SHA256SUMS").write_text(f"{hashlib.sha256((OUT / 'probe.json').read_bytes()).hexdigest()}  probe.json\n")
print(json.dumps(report, indent=2))
