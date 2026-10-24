#!/usr/bin/env python3
"""Convert an OPUS-MT (Marian) model from Hugging Face into the files PageWeave runs (decision D-025).

    python3 tools/convert_translate.py Helsinki-NLP/opus-mt-tc-big-fr-en <commit> OUTDIR [--quant int8|int4]

Writes OUTDIR/encoder.onnx, decoder.onnx (decoder merged with and without key/value cache, the
inputs `use_cache_branch` and `past_key_values.*` as PageWeave's runtime expects), quantized:
`int8` (default) dynamic 8-bit integers per channel; `int4` 4-bit weights in blocks of 32
(MatMulNBits) with 8-bit embeddings, about half the size. tokenizer.json (the source SentencePiece model's pieces
and scores in the vocabulary's id order; pieces the source model does not have get -100) and
config.json. Needs: torch (CPU), transformers<5, optimum[onnxruntime], sentencepiece, onnx-ir.
"""
import json
import os
import shutil
import sys
import tempfile


def tokenizer_json(model_dir: str, out: str) -> None:
    import sentencepiece as spm

    vocab = json.load(open(os.path.join(model_dir, "vocab.json"), encoding="utf-8"))
    sp = spm.SentencePieceProcessor(model_file=os.path.join(model_dir, "source.spm"))
    scores = {sp.id_to_piece(i): sp.get_score(i) for i in range(sp.get_piece_size())}
    by_id = sorted(vocab.items(), key=lambda kv: kv[1])
    assert [i for _, i in by_id] == list(range(len(by_id))), "vocabulary ids are not dense"
    specials = {"</s>", "<unk>", "<pad>"}
    pieces = []
    for piece, _ in by_id:
        if piece in specials:
            pieces.append([piece, 0.0])
        elif piece in scores:
            pieces.append([piece, float(scores[piece])])
        else:
            pieces.append([piece, -100.0])
    added = [
        {"id": vocab[t], "special": True, "content": t, "single_word": False, "lstrip": False, "rstrip": False, "normalized": False}
        for t in ("</s>", "<unk>", "<pad>")
        if t in vocab
    ]
    doc = {
        "version": "1.0",
        "truncation": None,
        "padding": None,
        "added_tokens": added,
        "normalizer": {"type": "Precompiled", "precompiled_charsmap": None},
        "pre_tokenizer": {"type": "Sequence", "pretokenizers": [{"type": "WhitespaceSplit"}, {"type": "Metaspace", "replacement": "▁", "add_prefix_space": True}]},
        "post_processor": None,
        "decoder": {"type": "Metaspace", "replacement": "▁", "add_prefix_space": True},
        "model": {"type": "Unigram", "unk_id": vocab["<unk>"], "vocab": pieces},
    }
    with open(out, "w", encoding="utf-8") as f:
        json.dump(doc, f, ensure_ascii=False)


def main() -> int:
    args = sys.argv[1:]
    quant = "int8"
    if "--quant" in args:
        i = args.index("--quant")
        quant = args[i + 1]
        del args[i : i + 2]
    if len(args) != 3 or quant not in ("int8", "int4"):
        print(__doc__)
        return 2
    repo, commit, outdir = args
    from huggingface_hub import snapshot_download
    from optimum.onnxruntime import ORTModelForSeq2SeqLM, ORTQuantizer
    from optimum.onnxruntime.configuration import AutoQuantizationConfig

    src = snapshot_download(repo, revision=commit, allow_patterns=["*.json", "*.spm", "*.safetensors", "README.md"])
    os.makedirs(outdir, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        m = ORTModelForSeq2SeqLM.from_pretrained(src, export=True, use_merged=True)
        m.save_pretrained(tmp)
        for name, final in (("encoder_model.onnx", "encoder.onnx"), ("decoder_model_merged.onnx", "decoder.onnx")):
            if quant == "int8":
                qcfg = AutoQuantizationConfig.avx2(is_static=False, per_channel=True)
                qdir = os.path.join(tmp, "q_" + final)
                ORTQuantizer.from_pretrained(tmp, file_name=name).quantize(save_dir=qdir, quantization_config=qcfg)
                q = [f for f in os.listdir(qdir) if f.endswith(".onnx")]
                shutil.copy(os.path.join(qdir, q[0]), os.path.join(outdir, final))
            else:
                import onnx
                from onnxruntime.quantization import QuantType, quantize_dynamic
                from onnxruntime.quantization.matmul_nbits_quantizer import DefaultWeightOnlyQuantConfig, MatMulNBitsQuantizer

                model = onnx.load(os.path.join(tmp, name))
                cfg = DefaultWeightOnlyQuantConfig(block_size=32, is_symmetric=True, accuracy_level=4)
                q = MatMulNBitsQuantizer(model, block_size=32, is_symmetric=True, algo_config=cfg)
                q.process()
                p4 = os.path.join(tmp, "q4_" + final)
                q.model.save_model_to_file(p4, use_external_data_format=False)
                quantize_dynamic(p4, os.path.join(outdir, final), op_types_to_quantize=["Gather"], weight_type=QuantType.QUInt8, per_channel=False)
    cfg = json.load(open(os.path.join(src, "config.json"), encoding="utf-8"))
    cfg["_name_or_path"] = repo
    cfg["_pageweave_source_commit"] = commit
    json.dump(cfg, open(os.path.join(outdir, "config.json"), "w", encoding="utf-8"), indent=2)
    tokenizer_json(src, os.path.join(outdir, "tokenizer.json"))
    for f in sorted(os.listdir(outdir)):
        print(f, os.path.getsize(os.path.join(outdir, f)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
