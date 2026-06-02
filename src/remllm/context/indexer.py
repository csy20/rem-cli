"""Lightweight codebase indexer — chunks by function/class/component, embeds, and retrieves.

Embedding backends (auto-selected in order of preference):
  1. sentence-transformers (CPU-friendly, all-MiniLM-L6-v2, 384-dim)
  2. Ollama embeddings API (via /api/embed endpoint)
  3. SHA-256 pseudo-embeddings (deterministic fallback, no install needed)
"""

import hashlib
import json
import logging
import os
import re
import struct
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

_logger = logging.getLogger(__name__)

_EMBED_DIM = 384
_sentence_model = None


def _get_sentence_model():
    global _sentence_model
    if _sentence_model is not None:
        return _sentence_model
    try:
        from sentence_transformers import SentenceTransformer

        _sentence_model = SentenceTransformer("all-MiniLM-L6-v2")
        _logger.info("Using sentence-transformers backend (all-MiniLM-L6-v2)")
    except ImportError:
        _sentence_model = False
    except Exception as exc:
        _logger.warning("Failed to load sentence-transformers: %s", exc)
        _sentence_model = False
    return _sentence_model


def _embed_text_backend(text: str) -> list[float]:
    model = _get_sentence_model()
    if model and model is not False:
        try:
            import numpy as np

            emb = model.encode(text, convert_to_numpy=True)
            if isinstance(emb, np.ndarray):
                return emb.astype(float).tolist()
        except Exception:
            pass

    ollama_url = os.environ.get("OLLAMA_API_URL", "http://localhost:11434")
    try:
        import urllib.request

        import json as _json

        req = urllib.request.Request(
            f"{ollama_url}/api/embed",
            data=_json.dumps({"model": "nomic-embed-text", "input": text}).encode(),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=10) as resp:
            data = _json.loads(resp.read())
            emb = data.get("embeddings", [[]])[0]
            if emb:
                return emb
    except Exception:
        pass

    return _sha256_embed(text)


def _sha256_embed(text: str) -> list[float]:
    hash_bytes = hashlib.sha256(text.encode("utf-8")).digest()
    embedding = []
    for i in range(0, min(len(hash_bytes) - 3, _EMBED_DIM * 4), 4):
        val = struct.unpack(">f", hash_bytes[i : i + 4])[0]
        embedding.append(val)
    while len(embedding) < _EMBED_DIM:
        embedding.append(0.0)
    return embedding[:_EMBED_DIM]


@dataclass
class CodeChunk:
    path: str
    name: str
    chunk_type: str
    content: str
    start_line: int
    end_line: int
    embedding: Optional[list[float]] = None
    metadata: dict = field(default_factory=dict)


class CodebaseIndexer:
    def __init__(self, index_path: Optional[Path] = None):
        # Prefer .rem/ layout (co-located with memory.md) so that `rem` (Rust CLI) can
        # discover the index automatically for retrieval at scale. models/ kept for
        # backward compat with old pipeline runs.
        default = Path(".rem/codebase_index.json")
        self.index_path = index_path or default
        self.chunks: list[CodeChunk] = []

    def _chunk_file(self, file_path: Path) -> list[CodeChunk]:
        chunks = []
        content = file_path.read_text(encoding="utf-8", errors="replace")
        ext = file_path.suffix.lstrip(".")

        chunkers = {
            "ts": self._chunk_typescript,
            "tsx": self._chunk_typescript,
            "js": self._chunk_typescript,
            "jsx": self._chunk_typescript,
            "py": self._chunk_python,
            "prisma": self._chunk_prisma,
            "sql": self._chunk_sql,
            "json": self._chunk_json_config,
            "yaml": self._chunk_yaml,
            "yml": self._chunk_yaml,
            "css": self._chunk_css,
            "md": self._chunk_markdown,
        }

        chunker = chunkers.get(ext, self._chunk_generic)
        for chunk in chunker(file_path, content):
            chunk.path = str(file_path)
            try:
                chunk.embedding = self._embed_text(chunk.content)
            except Exception:
                _logger.warning(
                    "Failed to embed chunk %s from %s", chunk.name, file_path
                )
                chunk.embedding = None
            chunks.append(chunk)

        return chunks

    def _chunk_typescript(self, file_path: Path, content: str) -> list[CodeChunk]:
        chunks = []
        for match in re.finditer(
            r"(?:export\s+)?(?:async\s+)?(?:function|class|const)\s+(\w+)",
            content,
        ):
            start_line = content[: match.start()].count("\n") + 1
            name = match.group(1)
            context = self._extract_context(content, match.start())
            chunks.append(
                CodeChunk(
                    path=str(file_path),
                    name=name,
                    chunk_type="function"
                    if "function" in match.group(0).lower()
                    else "class"
                    if "class" in match.group(0).lower()
                    else "variable",
                    content=context,
                    start_line=start_line,
                    end_line=start_line + context.count("\n"),
                )
            )
        if not chunks:
            chunks = self._chunk_generic(file_path, content)
        return chunks

    def _chunk_python(self, file_path: Path, content: str) -> list[CodeChunk]:
        chunks = []
        for match in re.finditer(
            r"(?:async\s+)?(?:def|class)\s+(\w+)",
            content,
        ):
            start_line = content[: match.start()].count("\n") + 1
            name = match.group(1)
            context = self._extract_context(content, match.start())
            chunks.append(
                CodeChunk(
                    path=str(file_path),
                    name=name,
                    chunk_type="class" if "class" in match.group(0) else "function",
                    content=context,
                    start_line=start_line,
                    end_line=start_line + context.count("\n"),
                )
            )
        if not chunks:
            chunks = self._chunk_generic(file_path, content)
        return chunks

    def _chunk_prisma(self, file_path: Path, content: str) -> list[CodeChunk]:
        chunks = []
        for match in re.finditer(r"model\s+(\w+)\s*\{", content):
            start_line = content[: match.start()].count("\n") + 1
            name = match.group(1)
            context = self._extract_context(content, match.start())
            chunks.append(
                CodeChunk(
                    path=str(file_path),
                    name=name,
                    chunk_type="model",
                    content=context,
                    start_line=start_line,
                    end_line=start_line + context.count("\n"),
                )
            )
        if not chunks:
            chunks = self._chunk_generic(file_path, content)
        return chunks

    def _chunk_sql(self, file_path: Path, content: str) -> list[CodeChunk]:
        chunks = []
        statements = re.split(r";\s*\n", content)
        for i, stmt in enumerate(statements):
            stmt = stmt.strip()
            if not stmt or len(stmt) < 10:
                continue
            name = f"sql_block_{i + 1}"
            chunks.append(
                CodeChunk(
                    path=str(file_path),
                    name=name,
                    chunk_type="sql",
                    content=stmt,
                    start_line=0,
                    end_line=0,
                )
            )
        if not chunks:
            chunks = self._chunk_generic(file_path, content)
        return chunks

    def _chunk_json_config(self, file_path: Path, content: str) -> list[CodeChunk]:
        name = file_path.stem
        top_key = "config"
        try:
            data = json.loads(content)
            if isinstance(data, dict):
                top_key = list(data.keys())[0] if data else "config"
        except json.JSONDecodeError:
            pass
        return [
            CodeChunk(
                path=str(file_path),
                name=top_key,
                chunk_type="config",
                content=content[:3000],
                start_line=0,
                end_line=content.count("\n"),
            )
        ]

    def _chunk_yaml(self, file_path: Path, content: str) -> list[CodeChunk]:
        return [
            CodeChunk(
                path=str(file_path),
                name=file_path.stem,
                chunk_type="config",
                content=content[:3000],
                start_line=0,
                end_line=content.count("\n"),
            )
        ]

    def _chunk_css(self, file_path: Path, content: str) -> list[CodeChunk]:
        chunks = []
        for match in re.finditer(r"\.([a-zA-Z][\w-]+)\s*\{", content):
            name = match.group(1)
            context = self._extract_context(content, match.start())
            chunks.append(
                CodeChunk(
                    path=str(file_path),
                    name=name,
                    chunk_type="css_class",
                    content=context,
                    start_line=content[: match.start()].count("\n") + 1,
                    end_line=0,
                )
            )
        if not chunks:
            chunks = self._chunk_generic(file_path, content)
        return chunks

    def _chunk_markdown(self, file_path: Path, content: str) -> list[CodeChunk]:
        chunks = []
        for match in re.finditer(r"^#{1,3}\s+(.+)$", content, re.MULTILINE):
            name = match.group(1).strip()
            start_line = content[: match.start()].count("\n") + 1
            context = self._extract_context(content, match.start())
            chunks.append(
                CodeChunk(
                    path=str(file_path),
                    name=name,
                    chunk_type="section",
                    content=context,
                    start_line=start_line,
                    end_line=0,
                )
            )
        if not chunks:
            chunks = self._chunk_generic(file_path, content)
        return chunks

    def _chunk_generic(self, file_path: Path, content: str) -> list[CodeChunk]:
        return [
            CodeChunk(
                path=str(file_path),
                name=file_path.name,
                chunk_type="file",
                content=content[:3000],
                start_line=0,
                end_line=content.count("\n"),
            )
        ]

    def _extract_context(
        self, content: str, start_pos: int, max_lines: int = 40
    ) -> str:
        pre = content[:start_pos]
        post = content[start_pos:]
        start_line_offset = pre.count("\n")
        post_lines = post.splitlines()
        block_lines = []
        indent_level = 0
        started = False

        for line in post_lines:
            stripped = line.strip()
            if not started:
                indent_level = len(line) - len(line.lstrip())
                started = True
            if (
                stripped
                and len(line) - len(line.lstrip()) < indent_level
                and "{" not in line
            ):
                break
            block_lines.append(line)
            if len(block_lines) >= max_lines:
                break

        return "\n".join(block_lines)

    def _embed_text(self, text: str) -> list[float]:
        return _embed_text_backend(text)

    def _cosine_sim(self, a: list[float], b: list[float]) -> float:
        dot = sum(x * y for x, y in zip(a, b))
        norm_a = (sum(x * x for x in a)) ** 0.5
        norm_b = (sum(x * x for x in b)) ** 0.5
        if norm_a == 0 or norm_b == 0:
            return 0.0
        return dot / (norm_a * norm_b)

    def index(self, project_dir: Path) -> int:
        self.chunks = []
        code_exts = {
            ".ts",
            ".tsx",
            ".js",
            ".jsx",
            ".py",
            ".prisma",
            ".sql",
            ".json",
            ".yaml",
            ".yml",
            ".css",
            ".md",
        }
        skip_dirs = {
            "node_modules",
            ".git",
            ".next",
            "dist",
            "build",
            "__pycache__",
            "venv",
            ".venv",
            "models",
            "target",
        }

        for file_path in project_dir.rglob("*"):
            if file_path.is_dir():
                continue
            if any(part in skip_dirs for part in file_path.parts):
                continue
            if file_path.suffix not in code_exts:
                continue
            if file_path.stat().st_size > 500_000:
                continue
            try:
                self.chunks.extend(self._chunk_file(file_path))
            except Exception:
                _logger.warning("Failed to chunk file: %s", file_path)
                continue

        self._save()
        return len(self.chunks)

    def _save(self) -> None:
        self.index_path.parent.mkdir(parents=True, exist_ok=True)
        data = {
            "chunks": [
                {
                    "path": c.path,
                    "name": c.name,
                    "chunk_type": c.chunk_type,
                    "content": c.content,
                    "start_line": c.start_line,
                    "end_line": c.end_line,
                    "embedding": c.embedding,
                }
                for c in self.chunks
            ]
        }
        self.index_path.write_text(json.dumps(data, indent=2), encoding="utf-8")

    def _load(self) -> None:
        if not self.index_path.exists():
            self.chunks = []
            return
        data = json.loads(self.index_path.read_text("utf-8"))
        self.chunks = [
            CodeChunk(
                path=c["path"],
                name=c.get("name", ""),
                chunk_type=c.get("chunk_type", "unknown"),
                content=c.get("content", ""),
                start_line=c.get("start_line", 0),
                end_line=c.get("end_line", 0),
                embedding=c.get("embedding"),
            )
            for c in data.get("chunks", [])
        ]

    def retrieve(self, query: str, top_k: int = 5) -> list[CodeChunk]:
        if not self.chunks:
            self._load()

        if not self.chunks:
            return []

        query_embed = self._embed_text(query)

        scored = []
        for chunk in self.chunks:
            if chunk.embedding is None:
                continue
            sim = self._cosine_sim(query_embed, chunk.embedding)
            keyword_bonus = 0.0
            query_lower = query.lower()
            if any(
                word in chunk.content.lower()
                for word in query_lower.split()
                if len(word) > 2
            ):
                keyword_bonus = 0.2
            scored.append((sim + keyword_bonus, chunk))

        scored.sort(key=lambda x: x[0], reverse=True)
        return [chunk for _, chunk in scored[:top_k]]

    def build_context_prompt(
        self, query: str, top_k: int = 5, max_chars: int = 3000
    ) -> str:
        chunks = self.retrieve(query, top_k)
        if not chunks:
            return ""

        lines = ["Here is relevant code from the project:\n"]
        total_chars = 0

        for chunk in chunks:
            block = f"File: {chunk.path} ({chunk.chunk_type}: {chunk.name})\n```\n{chunk.content}\n```\n"
            if total_chars + len(block) > max_chars:
                break
            lines.append(block)
            total_chars += len(block)

        return "\n".join(lines)


def index_codebase(
    project_dir: Path, index_path: Optional[Path] = None
) -> CodebaseIndexer:
    indexer = CodebaseIndexer(index_path)
    count = indexer.index(project_dir)
    print(f"Indexed {count} code chunks from {project_dir}")
    return indexer
