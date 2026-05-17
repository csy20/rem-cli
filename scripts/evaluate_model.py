#!/usr/bin/env python3

import argparse
import ast
import json
import re
import subprocess
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


def detect_language(row: dict) -> str:
    text = " ".join(
        [
            str(row.get("instruction", "")),
            str(row.get("input", "")),
            str(row.get("output", "")),
        ]
    ).lower()
    if any(token in text for token in ["python", "def ", "pytest"]):
        return "python"
    if any(
        token in text for token in ["javascript", "typescript", "react", "function "]
    ):
        return "javascript"
    if "sql" in text or "select " in text:
        return "sql"
    if "go" in text or "func " in text:
        return "go"
    if "rust" in text or "fn " in text:
        return "rust"
    return "unknown"


def extract_code(text: str) -> str:
    fenced = re.findall(r"```(?:[a-zA-Z0-9_+-]+)?\n(.*?)```", text, flags=re.DOTALL)
    if fenced:
        return "\n\n".join(block.strip() for block in fenced if block.strip())
    return text.strip()


def looks_like_code(text: str) -> bool:
    lower = text.lower()
    code_tokens = ["def ", "function ", "select ", "class ", "return ", "{", "}"]
    return any(token in lower for token in code_tokens)


def keyword_overlap_score(expected: str, actual: str) -> float:
    expected_tokens = set(re.findall(r"[a-zA-Z_]{3,}", expected.lower()))
    actual_tokens = set(re.findall(r"[a-zA-Z_]{3,}", actual.lower()))
    if not expected_tokens:
        return 0.0
    overlap = len(expected_tokens & actual_tokens)
    return round(overlap / len(expected_tokens), 4)


def python_syntax_ok(code_text: str) -> bool:
    if not code_text.strip():
        return False
    try:
        ast.parse(code_text)
        return True
    except SyntaxError:
        return False


def js_like_syntax_ok(code_text: str) -> bool:
    if not code_text.strip():
        return False
    opens = {"(": ")", "{": "}", "[": "]"}
    closes = {")", "}", "]"}
    stack = []
    for ch in code_text:
        if ch in opens:
            stack.append(opens[ch])
        elif ch in closes:
            if not stack or stack.pop() != ch:
                return False
    return not stack


def sql_shape_ok(code_text: str) -> bool:
    lower = code_text.lower()
    if "select" in lower and "from" in lower:
        return True
    return any(
        keyword in lower
        for keyword in ["insert into", "update ", "delete from", "create table"]
    )


def score_response(row: dict, response: str) -> dict:
    language = detect_language(row)
    code_text = extract_code(response)
    expected_output = str(row.get("output", ""))

    non_empty = bool(response.strip())
    has_code = looks_like_code(response)
    fenced_blocks = len(re.findall(r"```", response)) // 2
    overlap = keyword_overlap_score(expected_output, response)

    syntax_ok = 0
    if language == "python":
        syntax_ok = int(python_syntax_ok(code_text))
    elif language == "javascript":
        syntax_ok = int(js_like_syntax_ok(code_text))
    elif language == "sql":
        syntax_ok = int(sql_shape_ok(code_text))

    quality_score = round(
        (0.35 * int(non_empty))
        + (0.25 * int(has_code))
        + (0.25 * syntax_ok)
        + (0.15 * overlap),
        4,
    )

    return {
        "language": language,
        "non_empty": int(non_empty),
        "has_code": int(has_code),
        "fenced_blocks": fenced_blocks,
        "keyword_overlap": overlap,
        "syntax_ok": syntax_ok,
        "quality_score": quality_score,
    }


def run_prompt(model_name: str, prompt: str) -> str:
    result = subprocess.run(
        ["ollama", "run", model_name, prompt],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or "ollama run failed")
    return result.stdout.strip()


def main():
    parser = argparse.ArgumentParser(description="Evaluate model on fixed eval set.")
    parser.add_argument("--config", default="config/config.yaml")
    parser.add_argument("--model", required=True, help="Ollama model name to evaluate")
    parser.add_argument("--report", required=True, help="Output report path")
    args = parser.parse_args()

    config_path = Path(args.config)
    root = config_path.parent.parent
    with config_path.open("r", encoding="utf-8") as handle:
        config = yaml.safe_load(handle)

    eval_path = root / config["data"]["eval_file"]
    rows = load_jsonl(eval_path)
    if not rows:
        raise ValueError(f"No eval rows found in {eval_path}")

    aggregates = {
        "non_empty": 0,
        "has_code": 0,
        "fenced_blocks": 0,
        "keyword_overlap": 0.0,
        "syntax_ok": 0,
        "quality_score": 0.0,
    }
    language_breakdown = {}
    samples = []

    for row in rows:
        prompt = row["instruction"]
        if row.get("input"):
            prompt = f"{prompt}\n\nContext:\n{row['input']}"
        response = run_prompt(args.model, prompt)
        metrics = score_response(row, response)

        for key in aggregates:
            aggregates[key] += metrics[key]

        lang = metrics["language"]
        if lang not in language_breakdown:
            language_breakdown[lang] = {
                "count": 0,
                "syntax_ok": 0,
                "quality_score": 0.0,
            }
        language_breakdown[lang]["count"] += 1
        language_breakdown[lang]["syntax_ok"] += metrics["syntax_ok"]
        language_breakdown[lang]["quality_score"] += metrics["quality_score"]

        samples.append(
            {
                "instruction": row["instruction"],
                "input": row.get("input", ""),
                "reference_excerpt": str(row.get("output", ""))[:300],
                "response_excerpt": response[:500],
                "metrics": metrics,
            }
        )

    total = len(rows)
    language_rates = {}
    for lang, values in language_breakdown.items():
        count = values["count"]
        language_rates[lang] = {
            "count": count,
            "syntax_ok_rate": round(values["syntax_ok"] / count, 4),
            "avg_quality_score": round(values["quality_score"] / count, 4),
        }

    report = {
        "model": args.model,
        "eval_file": str(eval_path),
        "num_examples": total,
        "aggregate": aggregates,
        "rates": {
            "non_empty_rate": round(aggregates["non_empty"] / total, 4),
            "has_code_rate": round(aggregates["has_code"] / total, 4),
            "avg_fenced_blocks": round(aggregates["fenced_blocks"] / total, 4),
            "avg_keyword_overlap": round(aggregates["keyword_overlap"] / total, 4),
            "syntax_ok_rate": round(aggregates["syntax_ok"] / total, 4),
            "avg_quality_score": round(aggregates["quality_score"] / total, 4),
        },
        "language_rates": language_rates,
        "samples": samples,
    }

    report_path = Path(args.report)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(f"Wrote report: {report_path}")
    print(json.dumps(report["rates"], indent=2))


if __name__ == "__main__":
    main()
