"""Theme registry: 6 themes, active-theme state, and the shared Rich console."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Optional

from rich.console import Console

from rem import config


@dataclass(frozen=True)
class Theme:
    """A single color theme. All fields are hex strings."""

    name: str
    bg: str
    surface: str
    border: str
    accent: str
    accent_dim: str
    text_muted: str
    text_faint: str
    pill_bg: str
    pill_border: str
    pill_text: str
    kbd_bg: str
    kbd_border: str
    kbd_text: str
    sys_color: str
    sel_bg: str
    sel_left: str
    cursor: str


THEMES: dict[str, Theme] = {
    "GHOST": Theme(
        name="GHOST",
        bg="#030303",
        surface="#0d0d0d",
        border="#181818",
        accent="#e8e8e8",
        accent_dim="#888888",
        text_muted="#444444",
        text_faint="#222222",
        pill_bg="#1a1a1a",
        pill_border="#2a2a2a",
        pill_text="#e8e8e8",
        kbd_bg="#111111",
        kbd_border="#1e1e1e",
        kbd_text="#333333",
        sys_color="#1e1e1e",
        sel_bg="#141414",
        sel_left="#e8e8e8",
        cursor="#888888",
    ),
    "PHOSPHOR": Theme(
        name="PHOSPHOR",
        bg="#030a04",
        surface="#050e06",
        border="#0d2010",
        accent="#3aff5a",
        accent_dim="#2a8040",
        text_muted="#1a4020",
        text_faint="#0d2010",
        pill_bg="#061409",
        pill_border="#0d2a10",
        pill_text="#3aff5a",
        kbd_bg="#061409",
        kbd_border="#0d2010",
        kbd_text="#1a4020",
        sys_color="#0d2010",
        sel_bg="#0a1e0c",
        sel_left="#3aff5a",
        cursor="#3aff5a",
    ),
    "MIST": Theme(
        name="MIST",
        bg="#0c0f14",
        surface="#0f1420",
        border="#1a2538",
        accent="#7ba8d4",
        accent_dim="#4a6a90",
        text_muted="#2a3a55",
        text_faint="#1a2538",
        pill_bg="#102040",
        pill_border="#1a3060",
        pill_text="#7ba8d4",
        kbd_bg="#0f1420",
        kbd_border="#1a2538",
        kbd_text="#2a3a55",
        sys_color="#1a2538",
        sel_bg="#102040",
        sel_left="#7ba8d4",
        cursor="#7ba8d4",
    ),
    "EMBER": Theme(
        name="EMBER",
        bg="#0f0b06",
        surface="#161008",
        border="#251a08",
        accent="#f0a030",
        accent_dim="#7a5520",
        text_muted="#2a2010",
        text_faint="#1e1508",
        pill_bg="#1e1408",
        pill_border="#302010",
        pill_text="#f0a030",
        kbd_bg="#161008",
        kbd_border="#251a08",
        kbd_text="#2a2010",
        sys_color="#251a08",
        sel_bg="#1e1408",
        sel_left="#f0a030",
        cursor="#f0a030",
    ),
    "SAKURA": Theme(
        name="SAKURA",
        bg="#080610",
        surface="#0c0a1a",
        border="#1a1438",
        accent="#d46fa0",
        accent_dim="#5a4888",
        text_muted="#2a2048",
        text_faint="#130e22",
        pill_bg="#14103a",
        pill_border="#201850",
        pill_text="#d46fa0",
        kbd_bg="#0c0a1a",
        kbd_border="#1a1438",
        kbd_text="#2a2048",
        sys_color="#1a1438",
        sel_bg="#14103a",
        sel_left="#d46fa0",
        cursor="#d46fa0",
    ),
    "PAPER": Theme(
        name="PAPER",
        bg="#f5f2eb",
        surface="#ede8df",
        border="#d0cabb",
        accent="#3a3228",
        accent_dim="#5a5248",
        text_muted="#a09888",
        text_faint="#c8c0b0",
        pill_bg="#e0d8cc",
        pill_border="#c8c0b0",
        pill_text="#3a3228",
        kbd_bg="#e5e0d5",
        kbd_border="#d0cabb",
        kbd_text="#a09888",
        sys_color="#c8c0b0",
        sel_bg="#ddd8cc",
        sel_left="#3a3228",
        cursor="#3a3228",
    ),
}


DEFAULT_THEME_NAME = "GHOST"

console: Console = Console()


def get_active_theme() -> Theme:
    """Return the theme currently persisted in config.json."""
    name = str(config.get("theme") or DEFAULT_THEME_NAME)
    return THEMES.get(name, THEMES[DEFAULT_THEME_NAME])


def set_active_theme(name: str) -> None:
    """Persist the given theme name to config.json."""
    if name not in THEMES:
        raise ValueError(f"Unknown theme: {name!r}. Available: {sorted(THEMES.keys())}")
    config.set_value("theme", name)


def list_theme_names() -> list[str]:
    """Return the sorted list of available theme names."""
    return sorted(THEMES.keys())


def get_console() -> Console:
    """Return the shared Rich console used by all UI modules."""
    return console
