#!/usr/bin/env python3

import argparse
import json
from pathlib import Path


def load_report(path: Path):
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def main():
    parser = argparse.ArgumentParser(
        description="Compare baseline and post-train reports."
    )
    parser.add_argument("--baseline", required=True)
    parser.add_argument("--post", required=True)
    parser.add_argument("--baseline-exec", default="")
    parser.add_argument("--post-exec", default="")
    args = parser.parse_args()

    baseline = load_report(Path(args.baseline))
    post = load_report(Path(args.post))

    metrics = [
        "non_empty_rate",
        "has_code_rate",
        "avg_fenced_blocks",
        "avg_keyword_overlap",
        "syntax_ok_rate",
        "avg_quality_score",
    ]
    print("Metric comparison")
    for key in metrics:
        b = baseline["rates"].get(key, 0.0)
        p = post["rates"].get(key, 0.0)
        delta = round(p - b, 4)
        print(f"- {key}: baseline={b} post={p} delta={delta:+.4f}")

    baseline_lang = baseline.get("language_rates", {})
    post_lang = post.get("language_rates", {})
    all_langs = sorted(set(baseline_lang) | set(post_lang))
    if all_langs:
        print("\nPer-language quality")
        for lang in all_langs:
            bq = baseline_lang.get(lang, {}).get("avg_quality_score", 0.0)
            pq = post_lang.get(lang, {}).get("avg_quality_score", 0.0)
            print(f"- {lang}: baseline={bq} post={pq} delta={pq - bq:+.4f}")

    if args.baseline_exec and args.post_exec:
        baseline_exec = load_report(Path(args.baseline_exec))
        post_exec = load_report(Path(args.post_exec))
        print("\nExecutable comparison")
        exec_metrics = ["executable_checked_rate", "executable_pass_rate"]
        for key in exec_metrics:
            b = baseline_exec.get("rates", {}).get(key, 0.0)
            p = post_exec.get("rates", {}).get(key, 0.0)
            print(f"- {key}: baseline={b} post={p} delta={p - b:+.4f}")

        baseline_exec_lang = baseline_exec.get("language_rates", {})
        post_exec_lang = post_exec.get("language_rates", {})
        exec_langs = sorted(set(baseline_exec_lang) | set(post_exec_lang))
        if exec_langs:
            print("\nPer-language executable pass rate")
            for lang in exec_langs:
                b_rate = baseline_exec_lang.get(lang, {}).get("exec_pass_rate", 0.0)
                p_rate = post_exec_lang.get(lang, {}).get("exec_pass_rate", 0.0)
                print(
                    f"- {lang}: baseline={b_rate} post={p_rate} delta={p_rate - b_rate:+.4f}"
                )


if __name__ == "__main__":
    main()
