"""Command registry: dataclass + REGISTRY list + handlers for all slash commands."""

from __future__ import annotations

import json
import subprocess
import sys
import termios
import time
import tty
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Optional

from rich import box
from rich.console import Group
from rich.panel import Panel
from rich.table import Table

from rem import config
from rem.ui.theme import (
    get_active_theme,
    get_console,
    list_theme_names,
    set_active_theme,
)


@dataclass(frozen=True)
class Command:
    """A single slash command entry."""

    name: str
    description: str
    shortcut: str
    handler: Callable[[], None]


THEME_DESCRIPTIONS: dict[str, str] = {
    "GHOST": "dark monochrome",
    "PHOSPHOR": "green terminal",
    "MIST": "blue calm",
    "EMBER": "warm orange",
    "SAKURA": "pink/purple",
    "PAPER": "light beige",
}


conversation_history: list[dict[str, str]] = []


def _read_raw_key() -> Optional[str]:
    """Read a single keypress from stdin in raw mode; None for ESC."""
    fd = sys.stdin.fileno()
    old_settings = termios.tcgetattr(fd)
    try:
        tty.setraw(fd)
        ch = sys.stdin.read(1)
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, old_settings)
    if ch == "\x1b":
        return None
    if ch in ("\r", "\n"):
        return None
    return ch


def _theme_picker_panel() -> Panel:
    """Render the numbered theme picker panel."""
    t = get_active_theme()
    console = get_console()
    lines = ["[bold]Pick a theme:[/]\n"]
    names = list_theme_names()
    for i, name in enumerate(names, start=1):
        desc = THEME_DESCRIPTIONS.get(name, "")
        lines.append(
            f"  [{t.accent}]{i}.[/] [{t.accent_dim}]{name:<9}[/]  [{t.text_muted}]{desc}[/]"
        )
    lines.append(f"\n  [{t.text_muted}]Press 1-6 to pick, ESC to cancel.[/]")
    body = "\n".join(lines)
    return Panel(
        body,
        border_style=t.border,
        box=box.HORIZONTALS,
        padding=(0, 1),
        title=f"[{t.accent}]THEMES[/]",
        title_align="left",
    )


def _model_picker_panel(models: list[str], current: str) -> Panel:
    """Render the numbered model picker panel."""
    t = get_active_theme()
    lines = ["[bold]Available models:[/]\n"]
    if not models:
        lines.append(f"  [{t.text_muted}](no models found via `ollama list`)[/]")
    for i, name in enumerate(models, start=1):
        marker = "*" if name == current else " "
        lines.append(
            f"  [{t.accent}]{i}.[/] [{t.accent_dim}]{name}[/]  [{t.text_muted}]{marker}[/]"
        )
    lines.append(
        f"\n  [{t.text_muted}]Enter a number and press Enter, or just Enter to cancel.[/]"
    )
    body = "\n".join(lines)
    return Panel(
        body,
        border_style=t.border,
        box=box.HORIZONTALS,
        padding=(0, 1),
        title=f"[{t.accent}]MODELS[/]",
        title_align="left",
    )


def _fetch_ollama_models() -> list[str]:
    """Run `ollama list` and parse the first column; empty list on failure."""
    try:
        result = subprocess.run(
            ["ollama", "list"],
            capture_output=True,
            text=True,
            timeout=5,
            check=False,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired, OSError):
        return []
    if result.returncode != 0:
        return []
    models: list[str] = []
    for line in result.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        upper = line.upper()
        if (
            upper.startswith("NAME")
            or upper.startswith("NAME\t")
            or upper.startswith("NAME ")
        ):
            continue
        first = line.split()[0]
        if first:
            models.append(first)
    return models


def _cmd_help() -> None:
    """Render a Rich table of all registered commands."""
    t = get_active_theme()
    console = get_console()
    table = Table(
        box=box.SIMPLE,
        show_header=True,
        header_style=f"bold {t.accent}",
        border_style=t.border,
        expand=False,
    )
    table.add_column("name", style=t.accent, no_wrap=True)
    table.add_column("description", style=t.text_muted)
    table.add_column("shortcut", style=t.kbd_text, justify="right")
    for cmd in REGISTRY:
        table.add_row(cmd.name, cmd.description, cmd.shortcut or "")
    console.print(table)


def _cmd_mode() -> None:
    """Toggle CHAT <-> CODE in config and re-render the header."""
    t = get_active_theme()
    console = get_console()
    current = str(config.get("mode") or "CHAT")
    new_mode = "CODE" if current == "CHAT" else "CHAT"
    config.set_value("mode", new_mode)
    console.print(f"[{t.accent}]Mode switched to {new_mode}.[/]")
    from rem.ui.header import render_header

    render_header(str(config.get("model") or "rem-coder"), new_mode)


def _cmd_theme() -> None:
    """Clear screen, show picker, read 1-6 or ESC, apply choice, re-render header."""
    t = get_active_theme()
    console = get_console()
    console.clear()
    console.print(_theme_picker_panel())
    key = _read_raw_key()
    if key is None:
        return
    if not key.isdigit():
        console.print(f"[{t.text_muted}]Cancelled.[/]")
        return
    idx = int(key) - 1
    names = list_theme_names()
    if idx < 0 or idx >= len(names):
        console.print(f"[{t.text_muted}]Cancelled.[/]")
        return
    chosen = names[idx]
    set_active_theme(chosen)
    t = get_active_theme()
    console.print(f"[{t.accent}]Theme switched to {chosen}.[/]")
    from rem.ui.header import render_header

    render_header(
        str(config.get("model") or "rem-coder"), str(config.get("mode") or "CHAT")
    )


def _cmd_model() -> None:
    """Fetch ollama list, show picker, save the chosen model to config."""
    t = get_active_theme()
    console = get_console()
    models = _fetch_ollama_models()
    current = str(config.get("model") or "")
    console.print(_model_picker_panel(models, current))
    try:
        raw = input().strip()
    except EOFError:
        return
    if not raw:
        console.print(f"[{t.text_muted}]Cancelled.[/]")
        return
    if not raw.isdigit():
        console.print(f"[{t.text_muted}]Invalid selection.[/]")
        return
    idx = int(raw) - 1
    if idx < 0 or idx >= len(models):
        console.print(f"[{t.text_muted}]Invalid selection.[/]")
        return
    chosen = models[idx]
    config.set_value("model", chosen)
    console.print(f"[{t.accent}]Model set to {chosen}.[/]")
    from rem.ui.header import render_header

    render_header(chosen, str(config.get("mode") or "CHAT"))


def _cmd_clear() -> None:
    """Empty the conversation history list."""
    t = get_active_theme()
    console = get_console()
    conversation_history.clear()
    console.print(f"[{t.accent}]History cleared.[/]")


def _cmd_save() -> None:
    """Persist conversation_history to rem_session_{timestamp}.json in cwd."""
    t = get_active_theme()
    console = get_console()
    ts = int(time.time())
    path = Path(f"rem_session_{ts}.json")
    payload = {
        "timestamp": ts,
        "history": list(conversation_history),
        "config": config.load_config(),
    }
    with path.open("w", encoding="utf-8") as handle:
        json.dump(payload, handle, indent=2)
    console.print(f"[{t.accent}]Saved to {path}.[/]")


def _cmd_exit() -> None:
    """Print goodbye and terminate the process."""
    t = get_active_theme()
    console = get_console()
    console.print(f"[{t.sys_color}]Goodbye.[/]")
    sys.exit(0)


REGISTRY: list[Command] = [
    Command(
        name="/help",
        description="show this help table",
        shortcut="?",
        handler=_cmd_help,
    ),
    Command(
        name="/mode",
        description="toggle CHAT <-> CODE",
        shortcut="m",
        handler=_cmd_mode,
    ),
    Command(
        name="/model",
        description="switch the active model",
        shortcut="M",
        handler=_cmd_model,
    ),
    Command(
        name="/theme",
        description="switch color theme",
        shortcut="t",
        handler=_cmd_theme,
    ),
    Command(
        name="/clear",
        description="clear conversation history",
        shortcut="c",
        handler=_cmd_clear,
    ),
    Command(
        name="/save",
        description="save session to JSON",
        shortcut="s",
        handler=_cmd_save,
    ),
    Command(name="/exit", description="quit rem", shortcut="q", handler=_cmd_exit),
]


def find_command(name: str) -> Optional[Command]:
    """Look up a Command by exact name in REGISTRY."""
    for cmd in REGISTRY:
        if cmd.name == name:
            return cmd
    return None
