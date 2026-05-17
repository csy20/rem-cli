#!/usr/bin/env python3

import argparse
import json
import random
from pathlib import Path

import yaml


REQUIRED_KEYS = ("instruction", "input", "output")


def load_jsonl(path: Path):
    rows = []
    with path.open("r", encoding="utf-8") as handle:
        for line_no, raw in enumerate(handle, start=1):
            raw = raw.strip()
            if not raw:
                continue
            try:
                item = json.loads(raw)
            except json.JSONDecodeError as exc:
                raise ValueError(f"{path}:{line_no} invalid JSON: {exc}") from exc
            rows.append(item)
    return rows


def validate_rows(rows):
    cleaned = []
    dropped = 0
    for index, row in enumerate(rows):
        if not all(key in row for key in REQUIRED_KEYS):
            dropped += 1
            continue
        instruction = str(row["instruction"]).strip()
        user_input = str(row["input"]).strip()
        output = str(row["output"]).strip()
        if not instruction or not output:
            dropped += 1
            continue
        if len(output) < 8:
            dropped += 1
            continue
        cleaned.append(
            {
                "instruction": instruction,
                "input": user_input,
                "output": output,
            }
        )
    if not cleaned:
        raise ValueError("All rows were dropped by quality checks.")
    return cleaned, dropped


def write_jsonl(path: Path, rows):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False) + "\n")


def main():
    parser = argparse.ArgumentParser(description="Prepare train/val/eval datasets.")
    parser.add_argument("--config", default="config/config.yaml")
    args = parser.parse_args()

    config_path = Path(args.config)
    with config_path.open("r", encoding="utf-8") as handle:
        config = yaml.safe_load(handle)

    root = config_path.parent.parent
    data_cfg = config["data"]
    seed = int(config["project"]["seed"])
    train_split = float(data_cfg["train_split"])

    raw_path = root / data_cfg["raw_file"]
    train_path = root / data_cfg["train_file"]
    val_path = root / data_cfg["val_file"]
    eval_path = root / data_cfg["eval_file"]

    raw_rows = load_jsonl(raw_path)
    cleaned_rows, dropped = validate_rows(raw_rows)

    random.Random(seed).shuffle(cleaned_rows)
    split_idx = max(1, int(len(cleaned_rows) * train_split))
    split_idx = min(split_idx, len(cleaned_rows) - 1)

    train_rows = cleaned_rows[:split_idx]
    val_rows = cleaned_rows[split_idx:]

    eval_size = min(max(1, len(val_rows)), 200)
    eval_rows = val_rows[:eval_size]

    write_jsonl(train_path, train_rows)
    write_jsonl(val_path, val_rows)
    write_jsonl(eval_path, eval_rows)

    print("Data preparation complete")
    print(f"Raw rows: {len(raw_rows)}")
    print(f"Dropped rows: {dropped}")
    print(f"Train rows: {len(train_rows)} -> {train_path}")
    print(f"Val rows: {len(val_rows)} -> {val_path}")
    print(f"Eval rows: {len(eval_rows)} -> {eval_path}")


if __name__ == "__main__":
    main()
