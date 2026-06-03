"""Multi-quant GGUF packaging helper.

After Day-3 SFT and Day-4 DPO + merge, the merged HF model is exported to
multiple GGUF quantizations (q4_k_m, q5_k_m, q8_0). For each quant, this
script packages a separate Ollama model so users can pick the size/quality
tradeoff.

Usage:
    python3 scripts/package_ollama_multi.py \\
        --base-name rem-coder-v2 \\
        --gguf-dir models/rem-coder-gguf \\
        --quants q4_k_m,q5_k_m,q8_0
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from remllm.export.ollama import package_ollama


def package_multi(
    base_name: str,
    gguf_dir: Path,
    quants: list[str],
    modelfile: Path = Path("Modelfile.trained"),
) -> dict:
    results: dict[str, str] = {}
    failures: dict[str, str] = {}
    for quant in quants:
        gguf_file = gguf_dir / f"rem-coder-{quant}.gguf"
        model_name = f"{base_name}-{quant.replace('_', '')}"
        if not gguf_file.exists():
            failures[quant] = f"missing: {gguf_file}"
            print(f"SKIP {quant}: {gguf_file} not found")
            continue
        try:
            package_ollama(model_name, gguf_file, modelfile)
            results[quant] = model_name
        except Exception as exc:
            failures[quant] = str(exc)
            print(f"FAIL {quant}: {exc}")
    summary = {
        "base_name": base_name,
        "gguf_dir": str(gguf_dir),
        "packaged": results,
        "failed": failures,
    }
    manifest_path = gguf_dir / "packaging_manifest.json"
    manifest_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(json.dumps(summary, indent=2))
    return summary


def main():
    parser = argparse.ArgumentParser(
        description="Package multiple GGUF quants into Ollama"
    )
    parser.add_argument("--base-name", default="rem-coder-v2")
    parser.add_argument("--gguf-dir", default="models/rem-coder-gguf")
    parser.add_argument("--quants", default="q4_k_m,q5_k_m,q8_0")
    parser.add_argument("--modelfile", default="Modelfile.trained")
    args = parser.parse_args()
    quants = [q.strip() for q in args.quants.split(",") if q.strip()]
    package_multi(
        base_name=args.base_name,
        gguf_dir=Path(args.gguf_dir),
        quants=quants,
        modelfile=Path(args.modelfile),
    )


if __name__ == "__main__":
    main()
