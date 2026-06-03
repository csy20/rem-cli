"""Unified CLI for rem-llm — coding LLM training pipeline."""

import argparse
import datetime
import json
import os
import subprocess
from pathlib import Path

from remllm import __version__
from remllm.agent import run_agent
from remllm.config import resolve_project_root
from remllm.data.loader import load_jsonl
from remllm.data.prepper import prepare_data
from remllm.eval.benchmark import benchmark_models
from remllm.eval.beginner_eval import BeginnerEvaluator
from remllm.eval.comparator import compare_reports
from remllm.eval.executable import ExecutableEvaluator
from remllm.eval.quality import QualityEvaluator
from remllm.eval.security_eval import SecurityEvaluator
from remllm.eval.suite import run_full_evaluation
from remllm.eval.typescript_eval import TypeScriptEvaluator


def _load_eval_rows(config_path: str, data_key: str = "eval_file"):
    import yaml

    path = Path(config_path)
    with path.open("r", encoding="utf-8") as handle:
        config = yaml.safe_load(handle)
    root = resolve_project_root(path, str(config["data"][data_key]))
    eval_path = root / config["data"][data_key]
    rows = load_jsonl(eval_path)
    if not rows:
        print(f"No eval rows found in {eval_path}")
    return rows, config, eval_path


def cmd_version(_args: argparse.Namespace) -> None:
    print(f"remllm v{__version__}")


def cmd_data_prepare(args: argparse.Namespace) -> None:
    prepare_data(Path(args.config), force=args.force)


def cmd_data_generate(args: argparse.Namespace) -> None:
    from remllm.data.generator import (
        ANALYSIS_TEMPLATES,
        BACKEND_TEMPLATES,
        BEGINNER_TEMPLATES,
        CONVERSATION_TEMPLATES,
        DEVOPS_TEMPLATES,
        LANG_TEMPLATES,
        MOBILE_TEMPLATES,
        NEXTJS_TEMPLATES,
        generate_conversation_dataset,
        generate_dataset,
        write_dataset,
    )

    if args.domain == "conversation":
        rows = generate_conversation_dataset(CONVERSATION_TEMPLATES, seed=args.seed)
        write_dataset(rows, Path(args.output))
        print(f"Generated {len(rows)} conversation examples -> {args.output}")
        return

    templates = (
        NEXTJS_TEMPLATES
        + BEGINNER_TEMPLATES
        + BACKEND_TEMPLATES
        + DEVOPS_TEMPLATES
        + MOBILE_TEMPLATES
        + ANALYSIS_TEMPLATES
        + LANG_TEMPLATES
    )
    if args.domain == "beginner":
        templates = BEGINNER_TEMPLATES
    elif args.domain == "nextjs":
        templates = NEXTJS_TEMPLATES
    elif args.domain == "backend":
        templates = BACKEND_TEMPLATES
    elif args.domain == "devops":
        templates = DEVOPS_TEMPLATES
    elif args.domain == "mobile":
        templates = MOBILE_TEMPLATES
    elif args.domain == "analysis":
        templates = ANALYSIS_TEMPLATES
    elif args.domain != "all":
        templates = [t for t in templates if t.domain == args.domain]
    rows = generate_dataset(templates, seed=args.seed)
    write_dataset(rows, Path(args.output))
    print(f"Generated {len(rows)} examples -> {args.output}")
    stats: dict = {}
    for row in rows:
        domain = row.get("domain", "unknown")
        diff = row.get("difficulty", "unknown")
        stats.setdefault(domain, {"count": 0, "diffs": {}})
        stats[domain]["count"] += 1
        stats[domain]["diffs"][diff] = stats[domain]["diffs"].get(diff, 0) + 1
    print("Distribution:")
    for domain, s in sorted(stats.items()):
        print(f"  {domain}: {s['count']} rows — {s['diffs']}")


def cmd_data_dedup(args: argparse.Namespace) -> None:
    from remllm.data.dedup import deduplicate

    deduplicate(
        Path(args.input),
        Path(args.output),
        near_dedup=args.near,
        threshold=args.threshold,
    )


def cmd_data_filter(args: argparse.Namespace) -> None:
    from remllm.data.filter import filter_by_perplexity

    filter_by_perplexity(
        Path(args.input),
        Path(args.output),
        model=args.model,
        threshold=args.threshold,
        max_samples=args.max_samples,
        timeout_s=args.timeout_s,
    )


def cmd_data_mix(args: argparse.Namespace) -> None:
    from remllm.data.mixer import mix_datasets

    datasets = {}
    for pair in args.datasets.split(","):
        if ":" in pair:
            name, path = pair.split(":", 1)
            datasets[name.strip()] = Path(path.strip())
    ratios = {}
    if args.ratios:
        for pair in args.ratios.split(","):
            if ":" in pair:
                name, ratio = pair.split(":", 1)
                ratios[name.strip()] = float(ratio.strip())
    mix_datasets(
        datasets,
        ratios,
        Path(args.output),
        target_size=args.target_size,
        seed=args.seed,
    )


def cmd_data_augment(args: argparse.Namespace) -> None:
    from remllm.data.augment import augment_dataset

    augment_dataset(
        Path(args.input), Path(args.output), factor=args.factor, seed=args.seed
    )


def cmd_data_curate(args: argparse.Namespace) -> None:
    from remllm.data.curator import CurateConfig, curate

    sources = [s.strip() for s in args.sources.split(",") if s.strip()]
    source_limits: dict[str, int] = {}
    if args.source_limits:
        for pair in args.source_limits.split(","):
            if ":" in pair:
                name, lim = pair.split(":", 1)
                source_limits[name.strip()] = int(lim.strip())

    cfg = CurateConfig(
        sources=sources,
        output_dir=Path(args.output_dir),
        cache_dir=Path(args.cache_dir) if args.cache_dir else None,
        target_train_size=args.target_size,
        train_split=args.train_split,
        val_split=args.val_split,
        eval_split=args.eval_split,
        seed=args.seed,
        near_dedup_threshold=args.near_threshold,
        heuristic_threshold=args.heuristic_threshold,
        source_limits=source_limits,
    )
    curate(cfg)


def cmd_data_sources_list(_args: argparse.Namespace) -> None:
    from remllm.data.curator import list_sources

    for s in list_sources():
        print(f"{s['name']}\t{s['default_limit']}\t{s['description']}")


def cmd_data_fetch(args: argparse.Namespace) -> None:
    from remllm.data.curator import fetch_hf_dataset

    n = fetch_hf_dataset(
        args.dataset,
        Path(args.output),
        split=args.split,
        max_samples=args.max_samples,
        config=args.config or None,
    )
    if n == 0:
        print(f"Failed to fetch {args.dataset}. Install `datasets` and check network.")
    else:
        print(f"Fetched {n} rows -> {args.output}")


def cmd_data_dpo(args: argparse.Namespace) -> None:
    from remllm.data.dpo_generator import DPOPairConfig, generate_dpo_pairs

    cfg = DPOPairConfig(
        input_path=Path(args.input),
        output_path=Path(args.output),
        generator_model=args.model,
        n_samples=args.n_samples,
        temperature=args.temperature,
        max_prompts=args.max_prompts,
        seed=args.seed,
        min_score_gap=args.min_gap,
        timeout_s=args.timeout_s,
        base_url=args.ollama_url,
        num_predict=args.num_predict,
        skip_non_code=args.code_only,
        require_exec_judge=args.require_exec,
    )
    generate_dpo_pairs(cfg)


def cmd_data_distill_v2(args: argparse.Namespace) -> None:
    from remllm.data.distill_v2 import DistillConfig, distill_dataset_v2

    cfg = DistillConfig(
        input_path=Path(args.input),
        output_path=Path(args.output),
        teacher_model=args.teacher,
        student_model=args.student,
        n_samples=args.n_samples,
        temperature=args.temperature,
        max_samples=args.max_samples,
        seed=args.seed,
        timeout_s=args.timeout_s,
        base_url=args.ollama_url,
        num_predict=args.num_predict,
    )
    distill_dataset_v2(cfg)


def cmd_data_score_difficulty(args: argparse.Namespace) -> None:
    from remllm.data.difficulty import annotate_difficulty

    stats = annotate_difficulty(
        Path(args.input),
        Path(args.output),
        adaptive=not args.fixed_bands,
    )
    print(json.dumps(stats, indent=2))


def cmd_train_curriculum(args: argparse.Namespace) -> None:
    """Split data into 3 curriculum stages and write per-stage JSONLs + manifest.

    The actual GPU training is then invoked per-stage:
        for s in 1 2 3; do python3 -m remllm.train.unsloth --config ... ; done
    """
    from remllm.data.loader import load_jsonl
    from remllm.train.unsloth import split_curriculum_stages

    rows = load_jsonl(Path(args.input))
    manifest = split_curriculum_stages(
        rows,
        Path(args.output_dir),
        stages=tuple(args.stages.split(",")),
        seed=args.seed,
    )
    manifest_path = Path(args.output_dir) / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"Wrote curriculum manifest → {manifest_path}")
    print(json.dumps(manifest, indent=2))


def cmd_train_dpo_v2(args: argparse.Namespace) -> None:
    """Run DPO training on the v2 preference dataset.

    The output is `models/rem-coder-dpo/` (a LoRA adapter) that can be
    merged on top of the base SFT adapter from Day 3.
    """
    from remllm.config import load_config_dict
    from remllm.train.dpo import train_dpo

    config_path = Path(args.config)
    cfg = load_config_dict(config_path)
    cfg.setdefault("training", {})["dpo_enabled"] = True
    cfg["data"]["dpo_file"] = args.dpo_file
    if "dpo_output_dir" not in cfg["training"]:
        cfg["training"]["dpo_output_dir"] = "models/rem-coder-dpo"
    cfg["training"]["dpo_epochs"] = args.epochs
    cfg["training"]["dpo_beta"] = args.beta
    cfg["training"]["dpo_learning_rate"] = args.learning_rate
    cfg["training"]["dpo_learning_rate"] = args.learning_rate
    cfg["data"]["train_file"] = args.dpo_file
    if args.run_id:
        os.environ["RUN_ID"] = args.run_id
    train_dpo(config_path, run_id=args.run_id)


def cmd_eval_long_context(args: argparse.Namespace) -> None:
    """Run the long-context RoPE probe (Day-4 verification)."""
    from remllm.eval.long_context_probe import run_probe_suite

    targets = [int(t) for t in args.targets.split(",") if t.strip()]
    run_probe_suite(
        model=args.model,
        token_targets=targets,
        timeout_s=args.timeout_s,
        ollama_url=args.ollama_url,
        output_path=Path(args.report) if args.report else None,
    )


def cmd_data_scrape(args: argparse.Namespace) -> None:
    from remllm.data.scraper import scrape

    sources = [s.strip() for s in args.sources.split(",")]
    scrape(sources, Path(args.output), language=args.language, count=args.count)


def cmd_eval_quality(args: argparse.Namespace) -> None:
    rows, _, _ = _load_eval_rows(args.config)
    if not rows:
        return
    evaluator = QualityEvaluator()
    report = evaluator.evaluate(args.model, rows, timeout_s=args.timeout_s or None)
    report.write(Path(args.report))
    print(json.dumps(report.rates, indent=2))


def cmd_eval_exec(args: argparse.Namespace) -> None:
    rows, _, _ = _load_eval_rows(args.config)
    if not rows:
        return
    evaluator = ExecutableEvaluator()
    report = evaluator.evaluate(args.model, rows, timeout_s=args.timeout_s)
    report.write(Path(args.report))
    print(json.dumps(report.rates, indent=2))


def cmd_eval_beginner(args: argparse.Namespace) -> None:
    rows, _, _ = _load_eval_rows(args.config)
    if not rows:
        return
    evaluator = BeginnerEvaluator()
    report = evaluator.evaluate(args.model, rows, timeout_s=args.timeout_s)
    report.write(Path(args.report))
    print(json.dumps(report.rates, indent=2))


def cmd_eval_security(args: argparse.Namespace) -> None:
    rows, _, _ = _load_eval_rows(args.config)
    if not rows:
        return
    evaluator = SecurityEvaluator()
    report = evaluator.evaluate(args.model, rows, timeout_s=args.timeout_s)
    report.write(Path(args.report))
    print(json.dumps(report.rates, indent=2))


def cmd_eval_typescript(args: argparse.Namespace) -> None:
    rows, _, _ = _load_eval_rows(args.config)
    if not rows:
        return
    evaluator = TypeScriptEvaluator()
    report = evaluator.evaluate(args.model, rows, timeout_s=args.timeout_s)
    report.write(Path(args.report))
    print(json.dumps(report.rates, indent=2))


def cmd_eval_all(args: argparse.Namespace) -> None:
    rows, _, _ = _load_eval_rows(args.config)
    if not rows:
        return
    run_full_evaluation(
        args.model,
        rows,
        Path(args.output_dir),
        prefix=args.prefix,
        timeout_s=args.timeout_s,
    )


def cmd_eval_benchmark(args: argparse.Namespace) -> None:
    model_names = [n.strip() for n in args.models.split(",") if n.strip()]
    report = benchmark_models(
        model_names, Path(args.eval_file), args.max_samples, args.timeout_s
    )
    report_path = Path(args.report)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(f"Wrote benchmark: {report_path}")
    print(json.dumps(report["models"], indent=2))


def cmd_compare(args: argparse.Namespace) -> None:
    compare_reports(
        Path(args.baseline),
        Path(args.post),
        Path(args.baseline_exec) if args.baseline_exec else None,
        Path(args.post_exec) if args.post_exec else None,
    )


def cmd_train(args: argparse.Namespace) -> None:
    from remllm.train.unsloth import train_unsloth

    train_unsloth(Path(args.config))


def cmd_train_grpo(args: argparse.Namespace) -> None:
    from remllm.train.grpo import train_grpo

    train_grpo(Path(args.config))


def cmd_train_distill(args: argparse.Namespace) -> None:
    from remllm.train.distill import distill_dataset

    distill_dataset(
        Path(args.input),
        Path(args.output),
        teacher_model=args.teacher,
        student_model=args.student,
        temperature=args.temperature,
        max_samples=args.max_samples,
    )


def cmd_merge(args: argparse.Namespace) -> None:
    from remllm.export.merge import merge_adapter

    merge_adapter(Path(args.config))


def cmd_export_gguf(args: argparse.Namespace) -> None:
    from remllm.export.gguf import export_gguf

    llama_cpp = Path(args.llama_cpp_path)
    quant_list = [q.strip() for q in args.quants.split(",") if q.strip()]
    export_gguf(Path(args.merged_dir), Path(args.gguf_dir), llama_cpp, quant_list)


def cmd_package(args: argparse.Namespace) -> None:
    from remllm.export.ollama import package_ollama

    package_ollama(args.model_name, Path(args.gguf_file), Path(args.modelfile))


def cmd_agent(args: argparse.Namespace) -> None:
    project_dir = Path(args.project_dir) if args.project_dir else None
    index_path = Path(args.index_path) if args.index_path else None

    result = run_agent(
        task=args.task,
        model=args.model,
        project_dir=project_dir,
        index_path=index_path,
        dry_run=not args.execute,
        timeout_s=args.timeout_s,
    )

    print(f"\n{'=' * 60}")
    print(
        f"Profile: {json.dumps(result.profile.to_dict() if result.profile else {}, indent=2)}"
    )
    print(f"Codebase chunks: {result.codebase_chunks}")
    print(f"\n--- Raw Response (first 500 chars) ---")
    print(result.raw_response[:500])

    if result.structured:
        print(f"\n--- Structured Plan ---")
        print(result.structured.plan)
        if result.structured.operations:
            print(
                f"\n--- File Operations ({'DRY RUN' if not args.execute else 'EXECUTED'}) ---"
            )
            for op in result.file_ops:
                print(f"  {op}")
        if result.structured.tool_calls:
            print(f"\n--- Tool Calls ---")
            for tc in result.structured.tool_calls:
                print(f"  {tc.name}: {json.dumps(tc.arguments)}")

    summary_path = Path(args.summary) if args.summary else None
    if summary_path:
        summary_path.parent.mkdir(parents=True, exist_ok=True)
        summary_path.write_text(
            json.dumps(result.to_dict(), indent=2), encoding="utf-8"
        )
        print(f"\nSummary written to {summary_path}")


def cmd_index(args: argparse.Namespace) -> None:
    project_dir = Path(args.project_dir)
    index_path = Path(args.index_path)
    # If user got the new .rem default (relative), resolve it inside the target project
    # so `remllm index /path/to/bigproj` writes /path/to/bigproj/.rem/codebase_index.json
    if str(index_path) in {".rem/codebase_index.json", "models/codebase_index.json"}:
        index_path = project_dir / index_path
    if args.chromadb:
        from remllm.indexing import index_to_chromadb

        count = index_to_chromadb(project_dir, str(index_path))
        print(f"Indexed {count} chunks to ChromaDB at {index_path}")
    else:
        from remllm.context.indexer import index_codebase

        indexer = index_codebase(project_dir, index_path)
        if args.query:
            chunks = indexer.retrieve(args.query, top_k=args.top_k)
            if not chunks:
                print("No relevant code chunks found.")
            else:
                for chunk in chunks:
                    print(
                        f"\n--- {chunk.path}:{chunk.start_line} ({chunk.chunk_type}: {chunk.name}) ---"
                    )
                    print(chunk.content[:500])
            print(f"\nRetrieved {len(chunks)} chunks from {len(indexer.chunks)} total")


def cmd_search(args: argparse.Namespace) -> None:
    from remllm.context.indexer import CodebaseIndexer

    indexer = CodebaseIndexer(Path(args.index_path))
    chunks = indexer.retrieve(args.query, top_k=args.top_k)
    if not chunks:
        print("No relevant code chunks found.")
        return
    if args.raw:
        for chunk in chunks:
            print(
                json.dumps(
                    {
                        "path": chunk.path,
                        "name": chunk.name,
                        "chunk_type": chunk.chunk_type,
                        "start_line": chunk.start_line,
                        "content": chunk.content[:300],
                    },
                    indent=2,
                )
            )
    else:
        for i, chunk in enumerate(chunks, 1):
            print(
                f"\n[{i}] {chunk.path}:{chunk.start_line} ({chunk.chunk_type}: {chunk.name})"
            )
            print(chunk.content[:500])
    print(f"\nRetrieved {len(chunks)} chunks")


def cmd_pipeline(args: argparse.Namespace) -> None:
    from remllm.logging import get_logger

    root = Path.cwd()
    config_file = root / args.config
    base_model = args.base_model
    trained_model = args.trained_model
    skip_deps = os.environ.get("SKIP_DEPS", "1").lower() in ("1", "true", "yes")
    skip_baseline = os.environ.get("SKIP_BASELINE_IF_EXISTS", "1").lower() in (
        "1",
        "true",
        "yes",
    )
    run_id = os.environ.get("RUN_ID", datetime.datetime.now().strftime("%Y%m%d-%H%M%S"))
    run_dir = root / "models" / "experiments" / run_id
    run_dir.mkdir(parents=True, exist_ok=True)

    log = get_logger(run_id=run_id, base_model=base_model, trained_model=trained_model)
    log.info("pipeline_start", phase="0/7")

    log.info("phase_install_deps", phase="1/7")
    if skip_deps:
        log.info("install_skipped", reason="SKIP_DEPS=1")
    else:
        subprocess.run(
            ["pip", "install", "-r", str(root / "requirements.txt")], check=False
        )

    log.info("phase_prepare_data", phase="2/7")
    prepare_data(config_file)

    baseline_report = root / "models" / "evals" / "baseline.json"
    baseline_exec_report = root / "models" / "evals" / "baseline_exec.json"

    log.info("phase_baseline_eval", phase="3/7")
    if skip_baseline and baseline_report.exists():
        log.info("baseline_skipped", reason="cached report exists")
    else:
        rows, _, _ = _load_eval_rows(str(config_file))
        if rows:
            QualityEvaluator().evaluate(base_model, rows).write(baseline_report)
            ExecutableEvaluator().evaluate(base_model, rows).write(baseline_exec_report)

    log.info("phase_qlora_training", phase="4/7")
    from remllm.train.unsloth import train_unsloth

    train_unsloth(config_file)

    log.info("phase_merge_adapter", phase="5/7")
    from remllm.export.merge import merge_adapter

    merge_adapter(config_file)

    log.info("phase_export_gguf", phase="6/7")
    llama_cpp = os.environ.get("LLAMA_CPP_PATH", "")
    if llama_cpp:
        from remllm.export.gguf import export_gguf

        quant_list = [
            q.strip() for q in os.environ.get("QUANT_LIST", "q4_k_m").split(",")
        ]
        export_gguf(
            root / "models/rem-coder-merged",
            root / "models/rem-coder-gguf",
            Path(llama_cpp),
            quant_list,
        )
        from remllm.export.ollama import package_ollama

        package_ollama(
            trained_model,
            root / "models/rem-coder-gguf" / f"rem-coder-{quant_list[0]}.gguf",
            root / "Modelfile.trained",
        )
    else:
        log.info("gguf_skipped", reason="LLAMA_CPP_PATH not set")

    log.info("phase_post_train_eval", phase="7/7")
    rows, _, _ = _load_eval_rows(str(config_file))
    post_report = root / "models/evals/post_train.json"
    post_exec_report = root / "models/evals/post_train_exec.json"
    if rows:
        QualityEvaluator().evaluate(trained_model, rows).write(post_report)
        ExecutableEvaluator().evaluate(trained_model, rows).write(post_exec_report)
    compare_reports(
        baseline_report, post_report, baseline_exec_report, post_exec_report
    )
    log.info("pipeline_complete")


def _start_server(args: argparse.Namespace) -> None:
    try:
        import uvicorn

        uvicorn.run(
            "remllm.api_server:app",
            host=args.host,
            port=args.port,
            reload=args.reload,
        )
    except ImportError:
        print("Install API dependencies: pip install fastapi uvicorn")


def main():
    parser = argparse.ArgumentParser(
        description="remllm — coding LLM training pipeline", prog="remllm"
    )
    parser.add_argument("--version", action="version", version=f"remllm v{__version__}")
    sub = parser.add_subparsers(dest="command", title="commands")

    # data
    dp = sub.add_parser("data", help="Data operations")
    dp_sub = dp.add_subparsers(dest="data_command")
    dp_prep = dp_sub.add_parser("prepare", help="Prepare train/val/eval datasets")
    dp_prep.add_argument("--config", default="config/config.yaml")
    dp_prep.add_argument("--force", action="store_true")
    dp_gen = dp_sub.add_parser("generate", help="Generate synthetic web dev dataset")
    dp_gen.add_argument("--output", default="data/domains/nextjs/raw/fullstack.jsonl")
    dp_gen.add_argument(
        "--domain",
        default="all",
        choices=[
            "all",
            "analysis",
            "ansible",
            "backend",
            "beginner",
            "cicd",
            "conversation",
            "csharp",
            "css",
            "devops",
            "docker",
            "express",
            "fastapi",
            "flask",
            "flutter",
            "go",
            "html",
            "java",
            "kotlin",
            "kubernetes",
            "mobile",
            "nextjs",
            "php",
            "prisma",
            "python",
            "react",
            "ruby",
            "rust",
            "swift",
            "terminal",
            "typescript",
            "vue",
        ],
    )
    dp_gen.add_argument("--seed", type=int, default=42)
    dp_dedup = dp_sub.add_parser("dedup", help="Deduplicate dataset rows")
    dp_dedup.add_argument("input", help="Input JSONL path")
    dp_dedup.add_argument("--output", required=True, help="Output JSONL path")
    dp_dedup.add_argument(
        "--near", action="store_true", help="Enable near-deduplication"
    )
    dp_dedup.add_argument(
        "--threshold", type=float, default=0.85, help="Jaccard threshold for near-dedup"
    )
    dp_filter = dp_sub.add_parser("filter", help="Filter by perplexity/quality score")
    dp_filter.add_argument("input", help="Input JSONL path")
    dp_filter.add_argument("--output", required=True, help="Output JSONL path")
    dp_filter.add_argument("--model", default="qwen2.5-coder:1.5b")
    dp_filter.add_argument(
        "--threshold", type=float, default=5.0, help="Minimum quality score"
    )
    dp_filter.add_argument(
        "--max-samples", type=int, default=0, help="Max samples to score"
    )
    dp_filter.add_argument("--timeout-s", type=int, default=60)
    dp_mix = dp_sub.add_parser(
        "mix", help="Mix multiple datasets with weighted sampling"
    )
    dp_mix.add_argument(
        "--datasets", required=True, help="Comma-separated name:path pairs"
    )
    dp_mix.add_argument("--ratios", default="", help="Comma-separated name:ratio pairs")
    dp_mix.add_argument("--output", required=True, help="Output JSONL path")
    dp_mix.add_argument("--target-size", type=int, default=0)
    dp_mix.add_argument("--seed", type=int, default=42)
    dp_aug = dp_sub.add_parser("augment", help="Augment dataset with code variations")
    dp_aug.add_argument("input", help="Input JSONL path")
    dp_aug.add_argument("--output", required=True, help="Output JSONL path")
    dp_aug.add_argument("--factor", type=int, default=3, help="Augmentation factor")
    dp_aug.add_argument("--seed", type=int, default=42)
    dp_curate = dp_sub.add_parser(
        "curate",
        help="Build the curated training set v1 from registered sources (Day-1 deliverable)",
    )
    dp_curate.add_argument(
        "--sources",
        default="local_synthetic,hf_codealpaca,hf_magicoder,hf_evol_code",
        help="Comma-separated source names (see `remllm data sources list`)",
    )
    dp_curate.add_argument(
        "--source-limits",
        default="",
        help="Per-source row caps, e.g. 'hf_codealpaca:20000,hf_magicoder:30000'",
    )
    dp_curate.add_argument("--output-dir", default="data/curated/v1")
    dp_curate.add_argument("--cache-dir", default="data/sources")
    dp_curate.add_argument("--target-size", type=int, default=30000)
    dp_curate.add_argument("--train-split", type=float, default=0.9)
    dp_curate.add_argument("--val-split", type=float, default=0.05)
    dp_curate.add_argument("--eval-split", type=float, default=0.05)
    dp_curate.add_argument("--seed", type=int, default=42)
    dp_curate.add_argument("--near-threshold", type=float, default=0.85)
    dp_curate.add_argument("--heuristic-threshold", type=float, default=5.0)
    dp_sources = dp_sub.add_parser("sources", help="List registered data sources")
    dp_sources_sub = dp_sources.add_subparsers(dest="sources_command")
    dp_sources_list = dp_sources_sub.add_parser("list", help="List available sources")
    dp_fetch = dp_sub.add_parser(
        "fetch", help="Download a HuggingFace dataset to data/sources/"
    )
    dp_fetch.add_argument(
        "dataset", help="HF dataset id, e.g. sahil2801/CodeAlpaca-20k"
    )
    dp_fetch.add_argument("--output", required=True, help="Output JSONL path")
    dp_fetch.add_argument("--split", default="train")
    dp_fetch.add_argument("--max-samples", type=int, default=0)
    dp_fetch.add_argument(
        "--config", default="", help="Optional HF dataset config name"
    )
    dp_scrape = dp_sub.add_parser("scrape", help="Scrape code from public sources")
    dp_scrape.add_argument(
        "--sources", default="github_trending", help="Comma-separated source names"
    )
    dp_scrape.add_argument("--language", default="python", help="Programming language")
    dp_scrape.add_argument("--count", type=int, default=10, help="Number of results")
    dp_scrape.add_argument(
        "--output", default="data/scraped.jsonl", help="Output JSONL path"
    )
    dp_dpo = dp_sub.add_parser(
        "dpo",
        help="Generate DPO preference pairs via sampling + quality+exec judge (Day-2)",
    )
    dp_dpo.add_argument(
        "--input", default="data/curated/v1/train.jsonl", help="Source prompts JSONL"
    )
    dp_dpo.add_argument(
        "--output", default="data/preferences/v1/dpo.jsonl", help="Output DPO pairs"
    )
    dp_dpo.add_argument("--model", default="qwen2.5-coder:1.5b", help="Generator model")
    dp_dpo.add_argument("--n-samples", type=int, default=4)
    dp_dpo.add_argument("--temperature", type=float, default=0.8)
    dp_dpo.add_argument("--max-prompts", type=int, default=10000)
    dp_dpo.add_argument("--seed", type=int, default=42)
    dp_dpo.add_argument(
        "--min-gap", type=float, default=0.1, help="Min chosen-rejected score gap"
    )
    dp_dpo.add_argument("--timeout-s", type=int, default=60)
    dp_dpo.add_argument("--num-predict", type=int, default=512)
    dp_dpo.add_argument(
        "--code-only", action="store_true", help="Skip non-code prompts"
    )
    dp_dpo.add_argument(
        "--require-exec",
        action="store_true",
        help="Only keep pairs where chosen passes exec check and rejected fails",
    )
    dp_dpo.add_argument(
        "--ollama-url", default="http://localhost:11434", help="Ollama base URL"
    )
    dp_distill = dp_sub.add_parser(
        "distill-v2",
        help="Generate teacher-distilled completions with temperature sampling",
    )
    dp_distill.add_argument("--input", required=True, help="Source prompts JSONL")
    dp_distill.add_argument("--output", required=True, help="Output JSONL path")
    dp_distill.add_argument("--teacher", default="qwen2.5-coder:7b")
    dp_distill.add_argument("--student", default="qwen2.5-coder:1.5b")
    dp_distill.add_argument("--n-samples", type=int, default=1)
    dp_distill.add_argument("--temperature", type=float, default=0.7)
    dp_distill.add_argument("--max-samples", type=int, default=0)
    dp_distill.add_argument("--seed", type=int, default=42)
    dp_distill.add_argument("--timeout-s", type=int, default=120)
    dp_distill.add_argument("--num-predict", type=int, default=1024)
    dp_distill.add_argument("--ollama-url", default="http://localhost:11434")
    dp_score = dp_sub.add_parser(
        "score-difficulty",
        help="Annotate each row with a difficulty score + band (Day-3)",
    )
    dp_score.add_argument("--input", required=True, help="Input JSONL")
    dp_score.add_argument(
        "--output", required=True, help="Output JSONL with difficulty field"
    )
    dp_score.add_argument(
        "--fixed-bands",
        action="store_true",
        help="Use fixed 0.33/0.66 bands (default: adaptive percentiles)",
    )

    # eval
    eq = sub.add_parser("eval", help="Evaluation operations")
    eq_sub = eq.add_subparsers(dest="eval_command")
    eq_qual = eq_sub.add_parser("quality", help="Run quality evaluation")
    eq_qual.add_argument("--config", default="config/config.yaml")
    eq_qual.add_argument("--model", required=True)
    eq_qual.add_argument("--report", required=True)
    eq_qual.add_argument("--timeout-s", type=int, default=None)
    eq_exec = eq_sub.add_parser("exec", help="Run executable evaluation")
    eq_exec.add_argument("--config", default="config/config.yaml")
    eq_exec.add_argument("--model", required=True)
    eq_exec.add_argument("--report", required=True)
    eq_exec.add_argument("--timeout-s", type=int, default=30)
    eq_beginner = eq_sub.add_parser(
        "beginner", help="Run beginner HTML/CSS/terminal evaluation"
    )
    eq_beginner.add_argument("--config", default="config/config.yaml")
    eq_beginner.add_argument("--model", required=True)
    eq_beginner.add_argument("--report", required=True)
    eq_beginner.add_argument("--timeout-s", type=int, default=30)
    eq_bench = eq_sub.add_parser("benchmark", help="Benchmark model latency/throughput")
    eq_bench.add_argument("--models", required=True)
    eq_bench.add_argument("--eval-file", default="data/eval.jsonl")
    eq_bench.add_argument("--max-samples", type=int, default=20)
    eq_bench.add_argument("--timeout-s", type=int, default=180)
    eq_bench.add_argument("--report", default="models/evals/benchmark.json")

    eq_sec = eq_sub.add_parser(
        "security", help="Run security vulnerability scan on model output"
    )
    eq_sec.add_argument("--config", default="config/config.yaml")
    eq_sec.add_argument("--model", required=True)
    eq_sec.add_argument("--report", required=True)
    eq_sec.add_argument("--timeout-s", type=int, default=30)

    eq_ts = eq_sub.add_parser(
        "typescript", help="Run TypeScript type-checking evaluation"
    )
    eq_ts.add_argument("--config", default="config/config.yaml")
    eq_ts.add_argument("--model", required=True)
    eq_ts.add_argument("--report", required=True)
    eq_ts.add_argument("--timeout-s", type=int, default=30)

    eq_all = eq_sub.add_parser(
        "all", help="Run full evaluation suite (quality + exec + security + beginner)"
    )
    eq_all.add_argument("--config", default="config/config.yaml")
    eq_all.add_argument("--model", required=True)
    eq_all.add_argument("--output-dir", default="models/evals")
    eq_all.add_argument("--prefix", default="eval")
    eq_all.add_argument("--timeout-s", type=int, default=30)
    eq_lc = eq_sub.add_parser(
        "long-context",
        help="Run the long-context RoPE probe (Day-4 verification)",
    )
    eq_lc.add_argument("--model", required=True)
    eq_lc.add_argument(
        "--targets",
        default="1024,2048,4096,6000,8000",
        help="Comma-separated token sizes to probe",
    )
    eq_lc.add_argument("--timeout-s", type=int, default=180)
    eq_lc.add_argument("--ollama-url", default="http://localhost:11434")
    eq_lc.add_argument("--report", default="")

    # compare
    cmp = sub.add_parser("compare", help="Compare baseline and post-train reports")
    cmp.add_argument("--baseline", required=True)
    cmp.add_argument("--post", required=True)
    cmp.add_argument("--baseline-exec", default="")
    cmp.add_argument("--post-exec", default="")

    # train
    tr = sub.add_parser("train", help="Run training")
    tr_sub = tr.add_subparsers(dest="train_command")
    tr_qlora = tr_sub.add_parser("qlora", help="Run QLoRA training")
    tr_qlora.add_argument("--config", default="config/config.yaml")
    tr_grpo = tr_sub.add_parser("grpo", help="Run GRPO reasoning training")
    tr_grpo.add_argument("--config", default="config/config.yaml")
    tr_curriculum = tr_sub.add_parser(
        "curriculum",
        help="Split data into 3 staged curriculum JSONLs (Day-3 deliverable)",
    )
    tr_curriculum.add_argument("--input", required=True, help="Scored training JSONL")
    tr_curriculum.add_argument(
        "--output-dir",
        default="models/curriculum/v1",
        help="Where to write stage JSONLs",
    )
    tr_curriculum.add_argument(
        "--stages",
        default="easy,intermediate,advanced",
        help="Comma-separated stage names",
    )
    tr_curriculum.add_argument("--seed", type=int, default=42)
    tr_dpo = tr_sub.add_parser(
        "dpo-v2",
        help="Run DPO training on Day-2 preference data (Day-4 deliverable)",
    )
    tr_dpo.add_argument("--config", default="config/config.yaml")
    tr_dpo.add_argument(
        "--dpo-file", default="data/preferences/v1/dpo.jsonl", help="Preference JSONL"
    )
    tr_dpo.add_argument("--epochs", type=int, default=1)
    tr_dpo.add_argument("--beta", type=float, default=0.1)
    tr_dpo.add_argument("--learning-rate", type=float, default=5e-6)
    tr_dpo.add_argument("--run-id", default="")
    tr_distill = tr_sub.add_parser("distill", help="Run knowledge distillation")
    tr_distill.add_argument("input", help="Input JSONL with instructions")
    tr_distill.add_argument("--output", required=True, help="Output JSONL path")
    tr_distill.add_argument("--teacher", default="qwen2.5-coder:7b")
    tr_distill.add_argument("--student", default="qwen2.5-coder:1.5b")
    tr_distill.add_argument("--temperature", type=float, default=2.0)
    tr_distill.add_argument("--max-samples", type=int, default=100)

    # merge
    mg = sub.add_parser("merge", help="Merge LoRA adapter into base model")
    mg.add_argument("--config", default="config/config.yaml")

    # export
    ex = sub.add_parser("export", help="Export operations")
    ex_sub = ex.add_subparsers(dest="export_command")
    ex_gguf = ex_sub.add_parser("gguf", help="Export model to GGUF format")
    ex_gguf.add_argument("--merged-dir", default="models/rem-coder-merged")
    ex_gguf.add_argument("--gguf-dir", default="models/rem-coder-gguf")
    ex_gguf.add_argument("--quants", default="q4_k_m")
    ex_gguf.add_argument(
        "--llama-cpp-path", default=os.environ.get("LLAMA_CPP_PATH", "")
    )

    # package
    pkg = sub.add_parser("package", help="Package model into Ollama")
    pkg.add_argument("--model-name", default="rem-coder-trained")
    pkg.add_argument(
        "--gguf-file", default="models/rem-coder-gguf/rem-coder-q4_k_m.gguf"
    )
    pkg.add_argument("--modelfile", default="Modelfile.trained")

    # index
    idx = sub.add_parser("index", help="Index codebase for semantic search")
    idx.add_argument("project_dir", help="Project directory to index")
    idx.add_argument(
        "--index-path",
        default=".rem/codebase_index.json",
        help="Output index JSON path (default: .rem/ so the Rust `rem` CLI discovers it automatically)",
    )
    idx.add_argument(
        "--query", default=None, help="Optional query to search after indexing"
    )
    idx.add_argument("--top-k", type=int, default=5, help="Number of results for query")
    idx.add_argument("--chromadb", action="store_true", help="Use ChromaDB backend")

    # search
    sr = sub.add_parser("search", help="Search indexed codebase")
    sr.add_argument("query", help="Search query")
    sr.add_argument(
        "--index-path",
        default=".rem/codebase_index.json",
        help="Index JSON path (prefers .rem/ layout for `rem` CLI)",
    )
    sr.add_argument("--top-k", type=int, default=5, help="Number of results")
    sr.add_argument("--raw", action="store_true", help="Output raw JSON")

    # agent
    ag = sub.add_parser("agent", help="Run coding agent with structured output")
    ag.add_argument("task", help="Task description for the agent")
    ag.add_argument("--model", default="qwen2.5-coder:1.5b")
    ag.add_argument("--project-dir", default=None, help="Project directory to index")
    ag.add_argument("--index-path", default=None, help="Path to codebase index JSON")
    ag.add_argument("--timeout-s", type=int, default=120)
    ag.add_argument(
        "--execute",
        action="store_true",
        help="Execute file operations (default: dry-run)",
    )
    ag.add_argument("--summary", default=None, help="Write summary JSON to path")

    # pipeline
    pl = sub.add_parser("pipeline", help="Run full 7-phase pipeline")
    pl.add_argument("--config", default="config/config.yaml")
    pl.add_argument("--base-model", default="qwen2.5-coder:1.5b")
    pl.add_argument("--trained-model", default="rem-coder-trained")

    # serve
    sv = sub.add_parser("serve", help="Start REST API server")
    sv.add_argument("--host", default="0.0.0.0")
    sv.add_argument("--port", type=int, default=8080)
    sv.add_argument("--reload", action="store_true")

    args = parser.parse_args()
    if args.command is None:
        parser.print_help()
        return

    command_map = {
        "data": lambda a: {
            "prepare": cmd_data_prepare,
            "generate": cmd_data_generate,
            "dedup": cmd_data_dedup,
            "filter": cmd_data_filter,
            "mix": cmd_data_mix,
            "augment": cmd_data_augment,
            "scrape": cmd_data_scrape,
            "curate": cmd_data_curate,
            "sources": lambda a2: {
                "list": cmd_data_sources_list,
            }.get(
                str(getattr(a2, "sources_command", "") or "list"),
                cmd_data_sources_list,
            )(a2),
            "fetch": cmd_data_fetch,
            "dpo": cmd_data_dpo,
            "distill-v2": cmd_data_distill_v2,
        }.get(str(getattr(a, "data_command", "") or ""), lambda _: dp.print_help())(a),
        "eval": lambda a: {
            "quality": cmd_eval_quality,
            "exec": cmd_eval_exec,
            "beginner": cmd_eval_beginner,
            "security": cmd_eval_security,
            "typescript": cmd_eval_typescript,
            "all": cmd_eval_all,
            "benchmark": cmd_eval_benchmark,
            "long-context": cmd_eval_long_context,
        }.get(str(getattr(a, "eval_command", "") or ""), lambda _: eq.print_help())(a),
        "compare": cmd_compare,
        "index": cmd_index,
        "search": cmd_search,
        "agent": cmd_agent,
        "train": lambda a: {
            "qlora": cmd_train,
            "grpo": cmd_train_grpo,
            "distill": cmd_train_distill,
            "curriculum": cmd_train_curriculum,
            "dpo-v2": cmd_train_dpo_v2,
        }.get(str(getattr(a, "train_command", "") or "qlora"), cmd_train)(a),
        "merge": cmd_merge,
        "export": lambda a: {"gguf": cmd_export_gguf}.get(
            str(getattr(a, "export_command", "") or ""), lambda _: ex.print_help()
        )(a),
        "package": cmd_package,
        "pipeline": cmd_pipeline,
        "serve": lambda a: _start_server(a),
    }

    handler = command_map.get(args.command)
    if handler:
        handler(args)
    else:
        parser.print_help()


if __name__ == "__main__":
    main()
