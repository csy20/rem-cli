#!/usr/bin/env python3

import argparse
from pathlib import Path

import yaml


def main():
    parser = argparse.ArgumentParser(description="Merge LoRA adapter into base model.")
    parser.add_argument("--config", default="config/config.yaml")
    args = parser.parse_args()

    try:
        import torch
        from peft import PeftModel
        from transformers import AutoModelForCausalLM, AutoTokenizer
    except ImportError as exc:
        raise SystemExit(
            "Missing dependencies for merging. Install with:\n"
            "pip install torch transformers peft"
        ) from exc

    config_path = Path(args.config)
    root = config_path.parent.parent
    with config_path.open("r", encoding="utf-8") as handle:
        config = yaml.safe_load(handle)

    base_name = config["model"]["base_model_hf"]
    adapter_dir = root / config["training"]["output_dir"]
    merged_dir = root / config["training"]["merged_output_dir"]
    merged_dir.mkdir(parents=True, exist_ok=True)

    if not adapter_dir.exists():
        raise SystemExit(f"Adapter directory not found: {adapter_dir}")

    print(f"Loading base model: {base_name}")
    base_model = AutoModelForCausalLM.from_pretrained(
        base_name,
        torch_dtype=torch.float16,
        device_map="auto",
    )
    tokenizer = AutoTokenizer.from_pretrained(base_name)

    print(f"Loading adapter: {adapter_dir}")
    peft_model = PeftModel.from_pretrained(base_model, str(adapter_dir))

    print("Merging adapter into base model")
    merged_model = peft_model.merge_and_unload()
    merged_model.save_pretrained(str(merged_dir))
    tokenizer.save_pretrained(str(merged_dir))

    print(f"Merged model written to: {merged_dir}")
    print("Note: convert to GGUF before direct Ollama FROM <file.gguf> usage.")


if __name__ == "__main__":
    main()
