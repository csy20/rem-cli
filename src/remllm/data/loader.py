"""Data loader utilities: JSONL loading, caching, fingerprinting, FIM formatting."""

import hashlib
import json
import re
import random
from pathlib import Path
from typing import Any

FIM_PREFIX = "<|fim_begin|>"
FIM_SUFFIX = "<|fim_hole|>"
FIM_MIDDLE = "<|fim_end|>"
QWEN_IM_START = "<|im_start|>"
QWEN_IM_END = "<|im_end|>"

REQUIRED_KEYS = ("instruction", "input", "output")


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def load_jsonl(path: Path) -> list[dict]:
    rows = []
    with path.open("r", encoding="utf-8") as handle:
        for line_no, raw in enumerate(handle, start=1):
            raw = raw.strip()
            if not raw:
                continue
            try:
                rows.append(json.loads(raw))
            except json.JSONDecodeError as exc:
                raise ValueError(f"{path}:{line_no} invalid JSON: {exc}") from exc
    return rows


def write_jsonl(path: Path, rows: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False) + "\n")


def count_lines(path: Path) -> int:
    with path.open("r", encoding="utf-8") as handle:
        return sum(1 for _ in handle)


def load_cache(path: Path) -> dict[str, Any]:
    if not path.exists():
        return {}
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def write_cache(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        json.dump(payload, handle, indent=2)


def validate_rows(rows: list[dict]) -> tuple[list[dict], int]:
    cleaned = []
    dropped = 0
    for row in rows:
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


def build_dataset_fingerprint(raw_path: Path, seed: int, train_split: float) -> dict:
    return {
        "raw_file": str(raw_path),
        "raw_sha256": file_sha256(raw_path),
        "seed": seed,
        "train_split": train_split,
        "required_keys": list(REQUIRED_KEYS),
        "schema_version": 2,
    }


def format_training_row(row: dict, chat_template: bool = False) -> dict:
    prompt = row["instruction"]
    if row.get("input"):
        prompt += f"\n\nContext:\n{row['input']}"
    if chat_template:
        text = (
            f"{QWEN_IM_START}system\nYou are REM, a helpful coding assistant.{QWEN_IM_END}\n"
            f"{QWEN_IM_START}user\n{prompt}{QWEN_IM_END}\n"
            f"{QWEN_IM_START}assistant\n{row['output']}{QWEN_IM_END}"
        )
    else:
        text = f"### Instruction:\n{prompt}\n\n### Response:\n{row['output']}"
    return {"text": text}


def _split_code_syntactically(code: str) -> list[tuple[int, int]]:
    boundaries = []
    for match in re.finditer(
        r"(?:^\s*(?:def |class |async def |export |function |const |let |var |func |fn |pub fn |impl ))|(?:^\s*# )|(?:^\s*// )",
        code,
        re.MULTILINE,
    ):
        boundaries.append(match.start())
    if not boundaries:
        boundaries = [0]
    blocks = []
    for i in range(len(boundaries)):
        start = boundaries[i]
        end = boundaries[i + 1] if i + 1 < len(boundaries) else len(code)
        blocks.append((start, end))
    return blocks


def _fim_split(code: str, fim_rate: float = 0.8) -> list[str]:
    blocks = _split_code_syntactically(code)
    if len(blocks) < 2:
        return [code]
    results = []
    for _ in range(min(3, len(blocks))):
        if random.random() > fim_rate:
            results.append(code)
            continue
        split_idx = random.randint(1, len(blocks) - 1)
        prefix_end = blocks[split_idx - 1][1]
        suffix_start = blocks[split_idx][0]
        if random.random() < 0.5:
            results.append(
                f"{FIM_PREFIX}{code[:prefix_end]}{FIM_SUFFIX}{code[suffix_start:]}{FIM_MIDDLE}{code[prefix_end:suffix_start]}"
            )
        else:
            results.append(
                f"{FIM_SUFFIX}{code[suffix_start:]}{FIM_PREFIX}{code[:prefix_end]}{FIM_MIDDLE}{code[prefix_end:suffix_start]}"
            )
    return results or [code]


def format_fim_row(row: dict) -> dict:
    code = row.get("output", "")
    fim_splits = _fim_split(code)
    text = ""
    for split_code in fim_splits:
        if FIM_PREFIX in split_code:
            text = split_code
            break
    if not text:
        text = code
    return {"text": text}


def load_conversation_jsonl(path: Path) -> list[dict]:
    rows = load_jsonl(path)
    conversations = []
    for row in rows:
        if "turns" in row and isinstance(row["turns"], list):
            conversations.append(row)
    return conversations


def format_conversation_row(row: dict) -> dict:
    turns = row.get("turns", [])
    if not turns:
        return {"text": ""}
    text_parts = []
    for turn in turns:
        role = turn.get("role", "user")
        content = turn.get("content", "")
        if role == "user":
            text_parts.append(f"{QWEN_IM_START}user\n{content}{QWEN_IM_END}")
        elif role == "assistant":
            text_parts.append(f"{QWEN_IM_START}assistant\n{content}{QWEN_IM_END}")
        elif role == "system":
            text_parts.append(f"{QWEN_IM_START}system\n{content}{QWEN_IM_END}")
        else:
            text_parts.append(f"### {role.capitalize()}:\n{content}")
    return {"text": "\n\n".join(text_parts)}
