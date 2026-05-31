"""Tests for remllm/data/loader.py and prepper.py"""

import json
from pathlib import Path

from remllm.data.loader import (
    load_jsonl,
    write_jsonl,
    count_lines,
    validate_rows,
    format_training_row,
    format_conversation_row,
    build_dataset_fingerprint,
    file_sha256,
    QWEN_IM_START,
    QWEN_IM_END,
)
from remllm.data.prepper import prepare_data


def test_load_jsonl(temp_jsonl, sample_rows):
    rows = load_jsonl(temp_jsonl)
    assert len(rows) == len(sample_rows)
    assert rows[0]["instruction"] == sample_rows[0]["instruction"]


def test_write_and_count(temp_dir, sample_rows):
    path = temp_dir / "output.jsonl"
    write_jsonl(path, sample_rows)
    assert path.exists()
    assert count_lines(path) == len(sample_rows)


def test_validate_rows_drops_invalid():
    rows = [
        {"instruction": "", "input": "", "output": "short"},
        {"instruction": "do x", "input": "", "output": "enough text here"},
    ]
    cleaned, dropped = validate_rows(rows)
    assert dropped == 1
    assert len(cleaned) == 1


def test_validate_rows_empty_output():
    rows = [
        {"instruction": "do x", "input": "", "output": "hi"},
        {"instruction": "do y", "input": "", "output": "long enough output here"},
    ]
    cleaned, dropped = validate_rows(rows)
    assert dropped == 1
    assert len(cleaned) == 1


def test_validate_rows_missing_keys():
    rows = [
        {"instruction": "do x"},
        {"instruction": "do y", "input": "", "output": "valid output here yes"},
    ]
    cleaned, dropped = validate_rows(rows)
    assert dropped == 1
    assert len(cleaned) == 1


def test_format_training_row():
    row = {"instruction": "write foo", "input": "", "output": "bar"}
    result = format_training_row(row)
    assert "### Instruction:" in result["text"]
    assert "### Response:" in result["text"]
    assert "write foo" in result["text"]
    assert "bar" in result["text"]


def test_format_training_row_with_input():
    row = {"instruction": "write foo", "input": "context here", "output": "bar"}
    result = format_training_row(row)
    assert "Context:" in result["text"]
    assert "context here" in result["text"]


def test_format_training_row_chat_template():
    row = {"instruction": "write foo", "input": "", "output": "bar"}
    result = format_training_row(row, chat_template=True)
    text = result["text"]
    assert f"{QWEN_IM_START}system" in text
    assert f"{QWEN_IM_END}" in text
    assert f"{QWEN_IM_START}user" in text
    assert f"{QWEN_IM_START}assistant" in text
    assert "write foo" in text
    assert "bar" in text
    assert text.endswith(QWEN_IM_END)
    assert "You are REM" in text


def test_format_conversation_row():
    row = {
        "turns": [
            {"role": "system", "content": "You are helpful."},
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi there!"},
        ]
    }
    result = format_conversation_row(row)
    text = result["text"]
    assert f"{QWEN_IM_START}system" in text
    assert f"{QWEN_IM_START}user" in text
    assert f"{QWEN_IM_START}assistant" in text
    assert "You are helpful." in text
    assert "Hello" in text
    assert "Hi there!" in text
    assert QWEN_IM_END in text


def test_format_conversation_row_empty():
    row = {"turns": []}
    result = format_conversation_row(row)
    assert result["text"] == ""


def test_format_conversation_row_unknown_role():
    row = {"turns": [{"role": "bot", "content": "beep"}]}
    result = format_conversation_row(row)
    assert "### Bot:" in result["text"]


def test_file_sha256(temp_dir):
    path = temp_dir / "sha.txt"
    path.write_text("hello", encoding="utf-8")
    sha = file_sha256(path)
    assert len(sha) == 64  # SHA-256 hex length


def test_build_dataset_fingerprint(temp_jsonl):
    fp = build_dataset_fingerprint(temp_jsonl, 42, 0.9)
    assert fp["seed"] == 42
    assert fp["train_split"] == 0.9
    assert fp["schema_version"] == 2
    assert "raw_sha256" in fp


def test_prepare_data_integration(temp_dir, temp_jsonl):
    config_path = temp_dir / "config.yaml"
    data_dir = temp_dir / "data"
    data_dir.mkdir(parents=True)

    import shutil

    shutil.copy(temp_jsonl, data_dir / "raw.jsonl")

    config = {
        "project": {"name": "test", "seed": 42},
        "data": {
            "raw_file": str(data_dir / "raw.jsonl"),
            "train_file": str(data_dir / "train.jsonl"),
            "val_file": str(data_dir / "val.jsonl"),
            "eval_file": str(data_dir / "eval.jsonl"),
            "train_split": 0.8,
        },
    }
    import yaml

    with config_path.open("w") as f:
        yaml.dump(config, f)

    prepare_data(config_path, force=True)

    assert (data_dir / "train.jsonl").exists()
    assert (data_dir / "val.jsonl").exists()
    assert (data_dir / "eval.jsonl").exists()

    train_rows = load_jsonl(data_dir / "train.jsonl")
    val_rows = load_jsonl(data_dir / "val.jsonl")
    assert len(train_rows) + len(val_rows) == 3
    assert len(train_rows) > 0
