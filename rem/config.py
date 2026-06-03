"""JSON config persistence with atomic writes for the rem CLI."""

from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any

CONFIG_DIR = Path.home() / ".config" / "rem"
CONFIG_PATH = CONFIG_DIR / "config.json"

DEFAULTS: dict[str, Any] = {
    "theme": "GHOST",
    "mode": "CHAT",
    "model": "rem-coder",
}


def _ensure_dir() -> None:
    CONFIG_DIR.mkdir(parents=True, exist_ok=True)


def load_config() -> dict[str, Any]:
    """Load config from disk, creating it with defaults if absent."""
    _ensure_dir()
    if not CONFIG_PATH.exists():
        save_config(DEFAULTS)
        return dict(DEFAULTS)
    try:
        with CONFIG_PATH.open("r", encoding="utf-8") as handle:
            data = json.load(handle)
    except (json.JSONDecodeError, OSError):
        save_config(DEFAULTS)
        return dict(DEFAULTS)
    merged: dict[str, Any] = dict(DEFAULTS)
    if isinstance(data, dict):
        merged.update({k: v for k, v in data.items() if k in DEFAULTS})
    return merged


def save_config(data: dict[str, Any]) -> None:
    """Persist config atomically by writing to a tmp file and renaming."""
    _ensure_dir()
    tmp_path = CONFIG_PATH.with_suffix(".json.tmp")
    payload = dict(DEFAULTS)
    payload.update({k: v for k, v in data.items() if k in DEFAULTS})
    with tmp_path.open("w", encoding="utf-8") as handle:
        json.dump(payload, handle, indent=2)
    os.replace(tmp_path, CONFIG_PATH)


def get(key: str) -> Any:
    """Read a single config key, falling back to defaults if missing."""
    return load_config().get(key, DEFAULTS.get(key))


def set_value(key: str, value: Any) -> None:
    """Update a single config key and persist immediately."""
    data = load_config()
    data[key] = value
    save_config(data)
