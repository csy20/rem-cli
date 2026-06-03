"""Thin Ollama HTTP client for sampling-based operations (DPO, distillation).

The `ollama run` CLI doesn't expose `temperature` or `n` sampling, so we
hit the local HTTP API at /api/generate directly. Single-flight per call,
no async loop — callers handle concurrency.
"""

from __future__ import annotations

import json
import os
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Optional


DEFAULT_OLLAMA_URL = os.environ.get("OLLAMA_API_URL", "http://localhost:11434")


@dataclass
class GenerationParams:
    model: str
    prompt: str
    temperature: float = 0.8
    top_p: float = 0.95
    top_k: int = 40
    seed: int = 0
    num_predict: int = 512
    stop: Optional[list[str]] = None
    system: Optional[str] = None


def ollama_generate(
    params: GenerationParams,
    base_url: str = DEFAULT_OLLAMA_URL,
    timeout_s: int = 120,
) -> str:
    """Run a single ollama generation. Returns the text response."""
    payload: dict = {
        "model": params.model,
        "prompt": params.prompt,
        "stream": False,
        "options": {
            "temperature": params.temperature,
            "top_p": params.top_p,
            "top_k": params.top_k,
            "seed": params.seed,
            "num_predict": params.num_predict,
        },
    }
    if params.stop:
        payload["options"]["stop"] = params.stop
    if params.system:
        payload["system"] = params.system

    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        f"{base_url}/api/generate",
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout_s) as resp:
            body = resp.read().decode("utf-8")
            payload = json.loads(body)
            return payload.get("response", "").strip()
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as exc:
        raise RuntimeError(f"ollama_generate failed: {exc}") from exc


def ollama_sample_n(
    params: GenerationParams,
    n: int,
    base_url: str = DEFAULT_OLLAMA_URL,
    timeout_s: int = 120,
) -> list[str]:
    """Sample N independent generations from the same prompt.

    Falls back to subprocess `ollama run` if the HTTP API is unreachable,
    or if N=1 (the CLI is fine for a single response).
    """
    if n <= 0:
        return []
    if n == 1:
        return [ollama_generate(params, base_url, timeout_s)]
    samples: list[str] = []
    for i in range(n):
        p = GenerationParams(
            model=params.model,
            prompt=params.prompt,
            temperature=params.temperature,
            top_p=params.top_p,
            top_k=params.top_k,
            seed=params.seed + i,
            num_predict=params.num_predict,
            stop=params.stop,
            system=params.system,
        )
        try:
            text = ollama_generate(p, base_url, timeout_s)
        except RuntimeError:
            text = ""
        if text:
            samples.append(text)
    return samples


def ollama_health(base_url: str = DEFAULT_OLLAMA_URL, timeout_s: int = 5) -> bool:
    """Check whether the Ollama server is reachable."""
    try:
        with urllib.request.urlopen(f"{base_url}/api/tags", timeout=timeout_s) as resp:
            return resp.status == 200
    except Exception:
        return False
