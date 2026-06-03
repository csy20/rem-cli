"""Deduplication pipeline for training data.

Supports exact dedup (SHA-256), near-dedup (Jaccard on n-gram tokens), and
MinHash-based near-dedup for streaming / large corpora.
"""

import hashlib
import json
import re
from pathlib import Path
from typing import Any, Iterator

_TOKEN_RE = re.compile(r"\w+")


def _tokenize(text: str) -> list[str]:
    return _TOKEN_RE.findall(text.lower())


def _token_ngrams(text: str, n: int = 3) -> set[str]:
    tokens = text.lower().split()
    if len(tokens) < n:
        return {text.lower()}
    return {" ".join(tokens[i : i + n]) for i in range(len(tokens) - n + 1)}


def _jaccard_sim(a: set[str], b: set[str]) -> float:
    if not a and not b:
        return 1.0
    inter = len(a & b)
    union = len(a | b)
    return inter / union if union > 0 else 0.0


def deduplicate_exact(rows: list[dict]) -> tuple[list[dict], int]:
    seen = set()
    deduped = []
    for row in rows:
        text = json.dumps(row, sort_keys=True, ensure_ascii=False)
        digest = hashlib.sha256(text.encode()).hexdigest()
        if digest not in seen:
            seen.add(digest)
            deduped.append(row)
    dropped = len(rows) - len(deduped)
    return deduped, dropped


def stream_deduplicate_exact(
    rows: Iterator[dict[str, Any]],
) -> Iterator[dict[str, Any]]:
    """Streaming exact dedup using rolling hash set."""
    seen: set[str] = set()
    for row in rows:
        text = json.dumps(row, sort_keys=True, ensure_ascii=False)
        digest = hashlib.sha256(text.encode()).hexdigest()
        if digest in seen:
            continue
        seen.add(digest)
        yield row


def deduplicate_near(
    rows: list[dict],
    threshold: float = 0.85,
    key: str = "output",
) -> tuple[list[dict], int]:
    scored = []
    for row in rows:
        text = row.get(key, "") or row.get("instruction", "")
        scored.append((_token_ngrams(text), row))

    deduped = []
    dropped = 0
    for i, (ngrams_i, row) in enumerate(scored):
        is_dup = False
        for j in range(i):
            ng_j, _ = scored[j]
            if _jaccard_sim(ngrams_i, ng_j) >= threshold:
                is_dup = True
                break
        if is_dup:
            dropped += 1
        else:
            deduped.append(row)

    return deduped, dropped


def deduplicate(
    input_path: Path,
    output_path: Path,
    near_dedup: bool = False,
    threshold: float = 0.85,
) -> dict:
    from remllm.data.loader import load_jsonl, write_jsonl

    rows = load_jsonl(input_path)
    original = len(rows)

    rows, exact_dropped = deduplicate_exact(rows)
    near_dropped = 0
    if near_dedup:
        if len(rows) > 2000:
            from remllm.data.dedup import deduplicate_minhash

            rows, near_dropped = deduplicate_minhash(
                rows, threshold=threshold, num_perm=64
            )
        else:
            rows, near_dropped = deduplicate_near(rows, threshold=threshold)

    write_jsonl(output_path, rows)
    stats = {
        "original": original,
        "exact_duplicates_removed": exact_dropped,
        "near_duplicates_removed": near_dropped,
        "remaining": len(rows),
        "minhash": bool(near_dedup and len(rows) > 2000),
    }
    print(json.dumps(stats, indent=2))
    return stats


# ── MinHash-based near-dedup (scales to large corpora) ─────────────────────


_MERSENNE_PRIME = (1 << 61) - 1
_MAX_HASH = (1 << 32) - 1


def _make_minhash_hashes(num_perm: int, seed: int) -> tuple[list[int], list[int]]:
    """Generate hash function parameters (a, b) for MinHash."""
    import random

    rng = random.Random(seed)
    a = [rng.randint(1, _MERSENNE_PRIME - 1) for _ in range(num_perm)]
    b = [rng.randint(0, _MERSENNE_PRIME - 1) for _ in range(num_perm)]
    return a, b


def _minhash_signature(
    tokens: list[str], num_perm: int, a: list[int], b: list[int]
) -> list[int]:
    """Compute MinHash signature for a token list."""
    if not tokens:
        return [0] * num_perm
    token_hashes = [hash(token) & _MAX_HASH for token in set(tokens)]
    sig = [_MERSENNE_PRIME] * num_perm
    for h in token_hashes:
        for i in range(num_perm):
            v = (a[i] * h + b[i]) % _MERSENNE_PRIME
            if v < sig[i]:
                sig[i] = v
    return sig


def _estimate_jaccard(sig1: list[int], sig2: list[int]) -> float:
    if not sig1 or not sig2:
        return 0.0
    return sum(1 for a, b in zip(sig1, sig2) if a == b) / len(sig1)


def deduplicate_minhash(
    rows: list[dict],
    threshold: float = 0.85,
    num_perm: int = 64,
    key: str = "output",
    seed: int = 42,
    bands: int = 16,
) -> tuple[list[dict], int]:
    """MinHash-based near-dedup with LSH banding. Average O(n) per row.

    Bands rows into `bands` hash bands; only compare against candidates that
    share a band. For threshold=0.85 and bands=16, rows of permutation 64
    gives a sharp cutoff around the target similarity.
    """
    rows_per_band = max(1, num_perm // bands)
    a, b = _make_minhash_hashes(num_perm, seed)
    band_buckets: list[dict[int, list[int]]] = [dict() for _ in range(bands)]
    signatures: list[list[int]] = []
    deduped: list[dict] = []
    dropped = 0
    for row in rows:
        text = (row.get(key) or row.get("instruction") or "").lower()
        tokens = _tokenize(text)
        sig = _minhash_signature(tokens, num_perm, a, b)
        candidate_idxs: set[int] = set()
        for band_idx in range(bands):
            start = band_idx * rows_per_band
            end = start + rows_per_band if band_idx < bands - 1 else num_perm
            band_hash = hash(tuple(sig[start:end]))
            for idx in band_buckets[band_idx].get(band_hash, []):
                candidate_idxs.add(idx)
        is_dup = False
        for idx in candidate_idxs:
            if _estimate_jaccard(sig, signatures[idx]) >= threshold:
                is_dup = True
                break
        if is_dup:
            dropped += 1
            continue
        new_idx = len(signatures)
        signatures.append(sig)
        for band_idx in range(bands):
            start = band_idx * rows_per_band
            end = start + rows_per_band if band_idx < bands - 1 else num_perm
            band_hash = hash(tuple(sig[start:end]))
            band_buckets[band_idx].setdefault(band_hash, []).append(new_idx)
        deduped.append(row)
    return deduped, dropped


def stream_deduplicate_minhash(
    rows_iter: Iterator[dict],
    threshold: float = 0.85,
    num_perm: int = 64,
    key: str = "output",
    seed: int = 42,
    bands: int = 16,
) -> Iterator[dict]:
    """Streaming LSH-banded MinHash dedup. Yields non-duplicate rows."""
    rows_per_band = max(1, num_perm // bands)
    a, b = _make_minhash_hashes(num_perm, seed)
    band_buckets: list[dict[int, list[int]]] = [dict() for _ in range(bands)]
    signatures: list[list[int]] = []
    for row in rows_iter:
        text = (row.get(key) or row.get("instruction") or "").lower()
        tokens = _tokenize(text)
        sig = _minhash_signature(tokens, num_perm, a, b)
        candidate_idxs: set[int] = set()
        for band_idx in range(bands):
            start = band_idx * rows_per_band
            end = start + rows_per_band if band_idx < bands - 1 else num_perm
            band_hash = hash(tuple(sig[start:end]))
            for idx in band_buckets[band_idx].get(band_hash, []):
                candidate_idxs.add(idx)
        is_dup = False
        for idx in candidate_idxs:
            if _estimate_jaccard(sig, signatures[idx]) >= threshold:
                is_dup = True
                break
        if is_dup:
            continue
        new_idx = len(signatures)
        signatures.append(sig)
        for band_idx in range(bands):
            start = band_idx * rows_per_band
            end = start + rows_per_band if band_idx < bands - 1 else num_perm
            band_hash = hash(tuple(sig[start:end]))
            band_buckets[band_idx].setdefault(band_hash, []).append(new_idx)
        yield row
