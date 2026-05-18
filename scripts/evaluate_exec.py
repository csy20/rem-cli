#!/usr/bin/env python3

import argparse
import ast
import json
import re
import shutil
import sqlite3
import subprocess
import tempfile
from pathlib import Path

import yaml


def load_jsonl(path: Path) -> list[dict]:
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
    return "unknown"


def extract_code(text: str) -> str:
    fenced = re.findall(r"```(?:[a-zA-Z0-9_+-]+)?\n(.*?)```", text, flags=re.DOTALL)
    if fenced:
        return "\n\n".join(block.strip() for block in fenced if block.strip())
    return text.strip()


def run_prompt(model_name: str, prompt: str, timeout_s: int) -> str:
    result = subprocess.run(
        ["ollama", "run", model_name, prompt],
        capture_output=True,
        text=True,
        check=False,
        timeout=timeout_s,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or "ollama run failed")
    return result.stdout.strip()


def check_python_exec(code_text: str, timeout_s: int) -> tuple[int, str]:
    if not code_text.strip():
        return 0, "empty_code"
    try:
        ast.parse(code_text)
    except SyntaxError as exc:
        return 0, f"syntax_error:{exc.msg}"

    with tempfile.TemporaryDirectory(prefix="rem_exec_py_") as temp_dir:
        script_path = Path(temp_dir) / "generated.py"
        script_path.write_text(code_text, encoding="utf-8")
        process = subprocess.run(
            ["python3", "-I", str(script_path)],
            capture_output=True,
            text=True,
            check=False,
            timeout=timeout_s,
        )
        if process.returncode != 0:
            stderr = process.stderr.strip().splitlines()
            detail = stderr[-1] if stderr else "runtime_error"
            return 0, detail
    return 1, "ok"


def check_javascript_exec(code_text: str, timeout_s: int) -> tuple[int | None, str]:
    if not code_text.strip():
        return 0, "empty_code"
    if shutil.which("node") is None:
        return None, "node_missing"

    with tempfile.TemporaryDirectory(prefix="rem_exec_js_") as temp_dir:
        script_path = Path(temp_dir) / "generated.js"
        script_path.write_text(code_text, encoding="utf-8")
        process = subprocess.run(
            ["node", "--check", str(script_path)],
            capture_output=True,
            text=True,
            check=False,
            timeout=timeout_s,
        )
        if process.returncode != 0:
            stderr = process.stderr.strip().splitlines()
            detail = stderr[-1] if stderr else "syntax_check_failed"
            return 0, detail
    return 1, "ok"


def parse_table_definitions(text: str) -> list[tuple[str, list[str]]]:
    definitions = []
    for table_name, columns_text in re.findall(
        r"table\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*\(([^)]*)\)",
        text,
        flags=re.IGNORECASE,
    ):
        raw_columns = [chunk.strip() for chunk in columns_text.split(",")]
        columns = []
        for raw_column in raw_columns:
            if not raw_column:
                continue
            column_name = raw_column.split()[0].strip()
            if re.fullmatch(r"[a-zA-Z_][a-zA-Z0-9_]*", column_name):
                columns.append(column_name)
        if columns:
            definitions.append((table_name, columns))
    return definitions


def check_sql_exec(code_text: str, context_text: str) -> tuple[int, str]:
    if not code_text.strip():
        return 0, "empty_code"
    statements = code_text.strip()
    table_definitions = parse_table_definitions(context_text)

    try:
        connection = sqlite3.connect(":memory:")
        cursor = connection.cursor()
        for table_name, columns in table_definitions:
            column_sql = ", ".join(f"{column_name} TEXT" for column_name in columns)
            cursor.execute(f"CREATE TABLE {table_name} ({column_sql})")
        cursor.execute("BEGIN")
        cursor.execute(statements)
        cursor.fetchall()
        connection.rollback()
        connection.close()
    except sqlite3.Error as exc:
        return 0, f"sql_error:{exc}"
    return 1, "ok"


def evaluate_row(row: dict, response: str, timeout_s: int) -> dict:
    language = detect_language(row)
    code_text = extract_code(response)

    executable_checked = 1
    executable_ok = 0
    detail = "unsupported_language"

    if language == "python":
        executable_ok, detail = check_python_exec(code_text, timeout_s)
    elif language == "javascript":
        js_ok, detail = check_javascript_exec(code_text, timeout_s)
        if js_ok is None:
            executable_checked = 0
            executable_ok = 0
        else:
            executable_ok = js_ok
    elif language == "sql":
        executable_ok, detail = check_sql_exec(code_text, str(row.get("input", "")))
    else:
        executable_checked = 0
        executable_ok = 0

    return {
        "language": language,
        "executable_checked": executable_checked,
        "executable_ok": executable_ok,
        "detail": detail,
    }


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Run executable evaluation on eval set."
    )
    parser.add_argument("--config", default="config/config.yaml")
    parser.add_argument("--model", required=True)
    parser.add_argument("--report", required=True)
    parser.add_argument("--timeout-s", type=int, default=30)
    args = parser.parse_args()

    config_path = Path(args.config)
    root = config_path.parent.parent
    with config_path.open("r", encoding="utf-8") as handle:
        config = yaml.safe_load(handle)

    eval_path = root / config["data"]["eval_file"]
    rows = load_jsonl(eval_path)
    if not rows:
        raise ValueError(f"No eval rows found in {eval_path}")

    totals = {
        "executable_checked": 0,
        "executable_ok": 0,
    }
    language_totals: dict[str, dict] = {}
    samples = []

    for row in rows:
        prompt = row["instruction"]
        if row.get("input"):
            prompt = f"{prompt}\n\nContext:\n{row['input']}"
        response = run_prompt(args.model, prompt, timeout_s=args.timeout_s)
        metrics = evaluate_row(row, response, timeout_s=args.timeout_s)

        totals["executable_checked"] += metrics["executable_checked"]
        totals["executable_ok"] += metrics["executable_ok"]

        language = metrics["language"]
        if language not in language_totals:
            language_totals[language] = {
                "count": 0,
                "executable_checked": 0,
                "executable_ok": 0,
            }
        language_totals[language]["count"] += 1
        language_totals[language]["executable_checked"] += metrics["executable_checked"]
        language_totals[language]["executable_ok"] += metrics["executable_ok"]

        samples.append(
            {
                "instruction": row["instruction"],
                "input": row.get("input", ""),
                "response_excerpt": response[:500],
                "metrics": metrics,
            }
        )

    total_rows = len(rows)
    checked = totals["executable_checked"]
    pass_rate = round(totals["executable_ok"] / checked, 4) if checked else 0.0

    language_rates = {}
    for language, values in language_totals.items():
        checked_count = values["executable_checked"]
        language_rates[language] = {
            "count": values["count"],
            "checked_rate": round(checked_count / values["count"], 4)
            if values["count"]
            else 0.0,
            "exec_pass_rate": round(values["executable_ok"] / checked_count, 4)
            if checked_count
            else 0.0,
        }

    report = {
        "model": args.model,
        "eval_file": str(eval_path),
        "num_examples": total_rows,
        "aggregate": totals,
        "rates": {
            "executable_checked_rate": round(checked / total_rows, 4),
            "executable_pass_rate": pass_rate,
        },
        "language_rates": language_rates,
        "samples": samples,
    }

    report_path = Path(args.report)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(f"Wrote executable report: {report_path}")
    print(json.dumps(report["rates"], indent=2))


if __name__ == "__main__":
    main()
