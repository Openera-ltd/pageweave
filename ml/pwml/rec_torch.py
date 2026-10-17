"""Convert the shipped PP-OCRv6 small recognizer (ONNX) into a trainable PyTorch module.

onnx2torch does not implement `auto_pad=SAME_UPPER`. The recognizer uses it only on 2x2 kernels with
stride 1, where SAME_UPPER means exactly one row/column of padding at the end, independent of input
size, so those nodes are rewritten with explicit pads before conversion. `check_fidelity` compares the
converted module with ONNX Runtime on the original graph.
"""
from __future__ import annotations

import onnx
from onnx import helper


def explicit_same_upper(model: onnx.ModelProto) -> int:
    """Rewrite SAME_UPPER pads in place where they are input-size independent. Returns nodes changed."""
    changed = 0
    for n in model.graph.node:
        if n.op_type not in ("Conv", "MaxPool", "AveragePool"):
            continue
        attrs = {a.name: helper.get_attribute_value(a) for a in n.attribute}
        ap = attrs.get("auto_pad", b"NOTSET")
        ap = ap.decode() if isinstance(ap, bytes) else ap
        if ap != "SAME_UPPER":
            continue
        strides = list(attrs.get("strides", [1, 1]))
        kernel = list(attrs["kernel_shape"])
        dil = list(attrs.get("dilations", [1, 1]))
        if any(s != 1 for s in strides):
            raise ValueError(f"{n.name}: SAME_UPPER with stride {strides} depends on input size")
        total = [(k - 1) * d for k, d in zip(kernel, dil)]
        begin = [t // 2 for t in total]
        end = [t - b for t, b in zip(total, begin)]
        keep = [a for a in n.attribute if a.name != "auto_pad"]
        del n.attribute[:]
        n.attribute.extend(keep)
        n.attribute.append(helper.make_attribute("pads", begin + end))
        changed += 1
    return changed


def load_trainable(path: str):
    import torch  # noqa: F401
    from onnx2torch import convert

    model = onnx.load(path)
    explicit_same_upper(model)
    return convert(model)


def check_fidelity(path: str, widths=(160, 320, 640), batch=2, seed=0) -> float:
    """Largest absolute difference between the converted module and ONNX Runtime."""
    import numpy as np
    import onnxruntime as ort
    import torch

    tm = load_trainable(path).eval()
    sess = ort.InferenceSession(path, providers=["CPUExecutionProvider"])
    name = sess.get_inputs()[0].name
    rng = np.random.default_rng(seed)
    worst = 0.0
    for w in widths:
        x = rng.uniform(-1, 1, size=(batch, 3, 48, w)).astype("float32")
        with torch.no_grad():
            y = tm(torch.from_numpy(x)).numpy()
        yo = sess.run(None, {name: x})[0]
        assert y.shape == yo.shape, (y.shape, yo.shape)
        worst = max(worst, float(np.abs(y - yo).max()))
    return worst


if __name__ == "__main__":
    import sys

    print(check_fidelity(sys.argv[1]))
