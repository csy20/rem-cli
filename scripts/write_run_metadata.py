#!/usr/bin/env python3

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

import yaml


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def load_json(path: Path) -> dict:
    if not path.exists():
        return {}
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def get_git_commit(root_dir: Path) -> str:
    process = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=str(root_dir),
        capture_output=True,
        text=True,
        check=False,
    )
    if process.returncode != 0:
        return "unknown"
    return process.stdout.strip() or "unknown"


def main() -> None:
    parser = argparse.ArgumentParser(description="Write experiment run metadata.")
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--base-model", required=True)
    parser.add_argument("--trained-model", required=True)
    parser.add_argument("--baseline-report", required=True)
    parser.add_argument("--post-report", required=True)
    parser.add_argument("--baseline-exec-report", required=True)
    parser.add_argument("--post-exec-report", required=True)
    parser.add_argument("--config-file", required=True)
    args = parser.parse_args()

    config_path = Path(args.config_file)
    root_dir = config_path.parent.parent
    run_dir = root_dir / "models" / "experiments" / args.run_id
    run_dir.mkdir(parents=True, exist_ok=True)

    with config_path.open("r", encoding="utf-8") as handle:
        config = yaml.safe_load(handle)

    baseline_report = Path(args.baseline_report)
    post_report = Path(args.post_report)
    baseline_exec_report = Path(args.baseline_exec_report)
    post_exec_report = Path(args.post_exec_report)
    raw_data_path = root_dir / config["data"]["raw_file"]

    payload = {
        "run_id": args.run_id,
        "git_commit": get_git_commit(root_dir),
        "models": {
            "base": args.base_model,
            "trained": args.trained_model,
        },
        "config": {
            "path": str(config_path),
            "sha256": file_sha256(config_path),
        },
        "dataset": {
            "raw_file": str(raw_data_path),
            "raw_sha256": file_sha256(raw_data_path)
            if raw_data_path.exists()
            else "missing",
        },
        "reports": {
            "baseline": str(baseline_report),
            "post": str(post_report),
            "baseline_exec": str(baseline_exec_report),
            "post_exec": str(post_exec_report),
        },
        "metrics": {
            "baseline_rates": load_json(baseline_report).get("rates", {}),
            "post_rates": load_json(post_report).get("rates", {}),
            "baseline_exec_rates": load_json(baseline_exec_report).get("rates", {}),
            "post_exec_rates": load_json(post_exec_report).get("rates", {}),
        },
    }

    metadata_path = run_dir / "metadata.json"
    metadata_path.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    print(f"Wrote run metadata: {metadata_path}")


if __name__ == "__main__":
    main()
