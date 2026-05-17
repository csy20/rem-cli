#!/usr/bin/env python3

import argparse
import json
from pathlib import Path

import yaml


def load_jsonl(path: Path):
    rows = []
    with path.open("r", encoding="utf-8") as handle:
        for raw in handle:
            raw = raw.strip()
            if raw:
                rows.append(json.loads(raw))
    return rows


def format_row(row):
    prompt = row["instruction"]
    if row.get("input"):
        prompt += f"\n\nContext:\n{row['input']}"
    return {"text": f"### Instruction:\n{prompt}\n\n### Response:\n{row['output']}"}


def main():
    parser = argparse.ArgumentParser(description="QLoRA training with Unsloth.")
    parser.add_argument("--config", default="config/config.yaml")
    args = parser.parse_args()

    try:
        from datasets import Dataset
        from trl import SFTTrainer
        from transformers import TrainingArguments
        from unsloth import FastLanguageModel
    except ImportError as exc:
        raise SystemExit(
            "Missing dependencies for Unsloth training. Install with:\n"
            "pip install unsloth transformers datasets trl accelerate bitsandbytes peft"
        ) from exc

    config_path = Path(args.config)
    root = config_path.parent.parent
    with config_path.open("r", encoding="utf-8") as handle:
        config = yaml.safe_load(handle)

    model_cfg = config["model"]
    data_cfg = config["data"]
    train_cfg = config["training"]

    train_rows = [format_row(row) for row in load_jsonl(root / data_cfg["train_file"])]
    val_rows = [format_row(row) for row in load_jsonl(root / data_cfg["val_file"])]

    train_ds = Dataset.from_list(train_rows)
    val_ds = Dataset.from_list(val_rows)

    model, tokenizer = FastLanguageModel.from_pretrained(
        model_name=model_cfg["base_model_hf"],
        max_seq_length=int(data_cfg["max_length"]),
        load_in_4bit=bool(train_cfg["load_in_4bit"]),
    )

    model = FastLanguageModel.get_peft_model(
        model,
        r=int(train_cfg["lora_rank"]),
        target_modules=[
            "q_proj",
            "k_proj",
            "v_proj",
            "o_proj",
            "gate_proj",
            "up_proj",
            "down_proj",
        ],
        lora_alpha=int(train_cfg["lora_alpha"]),
        lora_dropout=float(train_cfg["lora_dropout"]),
        bias="none",
        use_gradient_checkpointing="unsloth",
        random_state=int(config["project"]["seed"]),
    )

    output_dir = root / train_cfg["output_dir"]
    output_dir.mkdir(parents=True, exist_ok=True)

    trainer = SFTTrainer(
        model=model,
        tokenizer=tokenizer,
        train_dataset=train_ds,
        eval_dataset=val_ds,
        dataset_text_field="text",
        max_seq_length=int(data_cfg["max_length"]),
        args=TrainingArguments(
            output_dir=str(output_dir),
            num_train_epochs=float(train_cfg["epochs"]),
            per_device_train_batch_size=int(train_cfg["per_device_train_batch_size"]),
            gradient_accumulation_steps=int(train_cfg["gradient_accumulation_steps"]),
            learning_rate=float(train_cfg["learning_rate"]),
            logging_steps=10,
            evaluation_strategy="epoch",
            save_strategy="epoch",
            bf16=True,
            fp16=False,
            optim="paged_adamw_8bit",
            report_to="none",
            seed=int(config["project"]["seed"]),
        ),
    )

    trainer.train()
    model.save_pretrained(str(output_dir))
    tokenizer.save_pretrained(str(output_dir))
    print(f"Saved LoRA adapter to {output_dir}")


if __name__ == "__main__":
    main()
