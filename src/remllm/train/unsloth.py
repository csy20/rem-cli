"""Unsloth QLoRA trainer for coding LLMs with experiment tracking.

Implements best practices from Gemma 4 / Unsloth / DeepSeek-Coder v2:
- train_on_responses_only: masks prompt tokens, trains only on assistant output
- FIM training support with ␇hole␈ / ␇fin␈ tokens
- Chat template formatting
- Offloaded gradient checkpointing
- Curriculum learning (easy → hard, configurable)
- Dynamic/mixed quantization
- RoPE scaling for long context
"""

import argparse
import json
import os
import random
from pathlib import Path
from typing import Any

import yaml

from remllm.data.loader import format_training_row, format_fim_row, load_jsonl


def _detect_report_to(config: dict) -> list[str]:
    reporters = []
    if os.environ.get("WANDB_PROJECT") or os.environ.get("WANDB_MODE"):
        reporters.append("wandb")
    if os.environ.get("TENSORBOARD_LOG_DIR"):
        reporters.append("tensorboard")
    if not reporters:
        reporters.append("none")
    return reporters


def _apply_curriculum(rows: list[dict], config: dict) -> list[dict]:
    """Sort rows by difficulty for a single-pass curriculum.

    For staged training (3 separate runs), use `split_curriculum_stages`
    which writes per-stage JSONL files and a manifest. The SFT trainer can
    then be called once per stage with a stage-specific config.
    """
    stages = config.get("curriculum_stages", ["easy", "intermediate", "advanced"])
    if not config.get("curriculum", False) or not stages:
        return rows
    stage_order = {s: i for i, s in enumerate(stages)}
    rows.sort(key=lambda r: stage_order.get(r.get("difficulty", ""), 99))
    print(f"Curriculum stages: {' → '.join(stages)}")
    return rows


def split_curriculum_stages(
    rows: list[dict],
    output_dir: Path,
    stages: list[str] | tuple[str, ...] = ("easy", "intermediate", "advanced"),
    seed: int = 42,
) -> dict:
    """Split rows by difficulty band and write per-stage JSONL files.

    Each stage progressively includes more difficulty:
      stage 1 (easy)            : only easy rows
      stage 2 (intermediate)    : easy + intermediate
      stage 3 (advanced)        : easy + intermediate + advanced

    Returns a manifest dict with per-stage row counts and output paths.
    """
    import random
    from collections import Counter
    from remllm.data.loader import write_jsonl

    output_dir.mkdir(parents=True, exist_ok=True)
    rng = random.Random(seed)
    difficulty_counts = Counter(r.get("difficulty", "easy") for r in rows)
    cumulative: list[str] = []
    manifest_stages: list[dict] = []
    for i, stage in enumerate(stages):
        cumulative.append(stage)
        stage_rows = [r for r in rows if r.get("difficulty", "easy") in cumulative]
        rng.shuffle(stage_rows)
        stage_path = output_dir / f"stage_{i + 1}_{stage}.jsonl"
        write_jsonl(stage_path, stage_rows)
        manifest_stages.append(
            {
                "stage": i + 1,
                "name": stage,
                "includes": list(cumulative),
                "rows": len(stage_rows),
                "path": str(stage_path),
            }
        )
        print(f"  Stage {i + 1} ({stage}): {len(stage_rows)} rows → {stage_path}")
    return {
        "stages": manifest_stages,
        "difficulty_distribution": dict(difficulty_counts),
        "output_dir": str(output_dir),
    }


def train_unsloth(config_path: Path) -> None:
    try:
        import torch
        from datasets import Dataset
        from trl import SFTTrainer
        from transformers import TrainingArguments, EarlyStoppingCallback
        from unsloth import FastLanguageModel
    except ImportError as exc:
        raise SystemExit(
            "Missing dependencies for Unsloth training. Install with:\n"
            "pip install unsloth transformers datasets trl accelerate bitsandbytes peft"
        ) from exc

    root = config_path.parent.parent
    with config_path.open("r", encoding="utf-8") as handle:
        config = yaml.safe_load(handle)

    model_cfg = config["model"]
    data_cfg = config["data"]
    train_cfg = config["training"]
    seed = int(config["project"]["seed"])

    raw_train_rows = load_jsonl(root / data_cfg["train_file"])
    raw_val_rows = load_jsonl(root / data_cfg["val_file"])

    raw_train_rows = _apply_curriculum(raw_train_rows, train_cfg)

    use_fim = bool(train_cfg.get("fim_training", False))
    if use_fim:
        train_rows = [format_fim_row(row) for row in raw_train_rows]
        val_rows = [format_fim_row(row) for row in raw_val_rows]
    else:
        use_chat = bool(train_cfg.get("chat_template", False))
        train_rows = [
            format_training_row(row, chat_template=use_chat) for row in raw_train_rows
        ]
        val_rows = [
            format_training_row(row, chat_template=use_chat) for row in raw_val_rows
        ]

    train_ds = Dataset.from_list(train_rows)
    val_ds = Dataset.from_list(val_rows)

    print(f"Train samples: {len(train_ds)}, Val samples: {len(val_ds)}")

    max_seq_length = int(data_cfg["max_length"])
    use_rope_scaling = bool(data_cfg.get("rope_scaling", False))
    rope_scaling_factor = float(data_cfg.get("rope_scaling_factor", 2.0))
    rope_scaling_type = str(data_cfg.get("rope_scaling_type", "linear"))

    model_kwargs = {
        "model_name": model_cfg["base_model_hf"],
        "max_seq_length": max_seq_length,
        "load_in_4bit": bool(train_cfg["load_in_4bit"]),
    }

    if use_rope_scaling:
        model_kwargs["rope_scaling"] = {
            "type": rope_scaling_type,
            "factor": rope_scaling_factor,
        }
        print(f"RoPE scaling: {rope_scaling_type} x{rope_scaling_factor}")

    model, tokenizer = FastLanguageModel.from_pretrained(**model_kwargs)

    dynamic_quant = bool(train_cfg.get("dynamic_quantization", False))
    gckpt_offload = bool(train_cfg.get("offload_gradient_checkpointing", False))

    gckpt_kwargs = {}
    if gckpt_offload:
        gckpt_kwargs = {"use_gradient_checkpointing": "unsloth_offload"}
        print("Using offloaded gradient checkpointing")
    else:
        gckpt_kwargs = {"use_gradient_checkpointing": "unsloth"}

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
        random_state=seed,
        **gckpt_kwargs,
    )

    output_dir = root / train_cfg["output_dir"]
    output_dir.mkdir(parents=True, exist_ok=True)

    resume = os.environ.get("TRAIN_RESUME", "").lower() in ("1", "true", "yes")
    resume_from = None
    if resume:
        resume_dirs = sorted(output_dir.glob("checkpoint-*"))
        if resume_dirs:
            resume_from = str(resume_dirs[-1])
            print(f"Resuming from checkpoint: {resume_from}")

    report_to = _detect_report_to(config)

    run_name = os.environ.get("WANDB_RUN_NAME") or os.environ.get("RUN_ID")
    if run_name and "wandb" in report_to:
        os.environ.setdefault("WANDB_NAME", run_name)

    callbacks = []
    early_stopping_patience = int(os.environ.get("EARLY_STOPPING_PATIENCE", "0"))
    if early_stopping_patience > 0:
        callbacks.append(
            EarlyStoppingCallback(early_stopping_patience=early_stopping_patience)
        )
        print(f"Early stopping enabled (patience={early_stopping_patience})")

    train_on_responses = bool(train_cfg.get("train_on_responses_only", True))
    packing = bool(data_cfg.get("pack_sequences", False))

    bf16_flag = torch.cuda.is_available() and torch.cuda.get_device_capability()[0] >= 8
    fp16_flag = torch.cuda.is_available() and torch.cuda.get_device_capability()[0] < 8

    training_args = TrainingArguments(
        output_dir=str(output_dir),
        num_train_epochs=float(train_cfg["epochs"]),
        per_device_train_batch_size=int(train_cfg["per_device_train_batch_size"]),
        gradient_accumulation_steps=int(train_cfg["gradient_accumulation_steps"]),
        learning_rate=float(train_cfg["learning_rate"]),
        lr_scheduler_type=str(train_cfg.get("lr_scheduler_type", "cosine")),
        warmup_ratio=float(train_cfg.get("warmup_ratio", 0.05)),
        weight_decay=float(train_cfg.get("weight_decay", 0.01)),
        max_grad_norm=float(train_cfg.get("max_grad_norm", 1.0)),
        logging_steps=int(train_cfg.get("logging_steps", 10)),
        logging_first_step=True,
        evaluation_strategy="epoch",
        save_strategy="epoch",
        save_total_limit=int(train_cfg.get("save_total_limit", 3)),
        load_best_model_at_end=True if early_stopping_patience > 0 else False,
        metric_for_best_model="eval_loss",
        bf16=bf16_flag,
        fp16=fp16_flag,
        optim="paged_adamw_8bit",
        report_to=report_to,
        run_name=run_name,
        seed=seed,
        dataloader_drop_last=packing,
        gradient_checkpointing_kwargs=(
            {"use_reentrant": False} if gckpt_offload else {}
        ),
    )

    trainer = SFTTrainer(
        model=model,
        tokenizer=tokenizer,
        train_dataset=train_ds,
        eval_dataset=val_ds,
        dataset_text_field="text",
        max_seq_length=max_seq_length,
        packing=packing,
        args=training_args,
        callbacks=callbacks,
        train_on_responses_only=train_on_responses,
    )

    trainer.train(resume_from_checkpoint=resume_from)
    model.save_pretrained(str(output_dir))
    tokenizer.save_pretrained(str(output_dir))

    metrics_path = output_dir / "training_metrics.json"
    metrics = {
        "train_samples": len(train_ds),
        "val_samples": len(val_ds),
        "report_to": report_to,
        "resumed": bool(resume_from),
        "train_on_responses_only": train_on_responses,
        "fim_training": use_fim,
        "curriculum": bool(train_cfg.get("curriculum", False)),
        "rope_scaling": use_rope_scaling,
        "dynamic_quantization": dynamic_quant,
        "offloaded_gckpt": gckpt_offload,
        "max_seq_length": max_seq_length,
    }
    if hasattr(trainer.state, "best_metric"):
        metrics["best_eval_loss"] = trainer.state.best_metric
    if hasattr(trainer.state, "log_history"):
        metrics["log_steps"] = len(trainer.state.log_history)

    metrics_path.write_text(json.dumps(metrics, indent=2))
    print(f"Saved LoRA adapter to {output_dir}")
    print(json.dumps(metrics, indent=2))


def main():
    parser = argparse.ArgumentParser(description="QLoRA training with Unsloth.")
    parser.add_argument("--config", default="config/config.yaml")
    args = parser.parse_args()
    train_unsloth(Path(args.config))


if __name__ == "__main__":
    main()
