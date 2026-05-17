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


if __name__ == "__main__":
    main()
