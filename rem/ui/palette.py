"""Slash command palette: interactive command picker with live filter."""

from __future__ import annotations

from typing import Optional

from prompt_toolkit import PromptSession
from prompt_toolkit.formatted_text import HTML
from prompt_toolkit.history import InMemoryHistory
from prompt_toolkit.key_binding import KeyBindings

from rich import box
from rich.console import Console
from rich.live import Live
from rich.panel import Panel
from rich.table import Table

from rem.commands.registry import Command
from rem.ui.theme import get_active_theme, get_console


SHORTCUT_LABELS: dict[str, str] = {
    "/help": "↵",
    "/mode": "m",
    "/model": "",
    "/theme": "t",
    "/clear": "c",
    "/save": "s",
    "/exit": "q",
}


def _filter_commands(registry: list[Command], query: str) -> list[Command]:
    """Prefix-match on command name; case-insensitive."""
    q = query.lstrip("/").lower()
    if not q:
        return list(registry)
    return [c for c in registry if c.name.lstrip("/").lower().startswith(q)]


def _render_palette(
    console: Console,
    matches: list[Command],
    selected_idx: int,
    query: str,
) -> Panel:
    t = get_active_theme()
    grid = Table.grid(expand=True, padding=(0, 1))
    grid.add_column("selector", width=2)
    grid.add_column("name", width=10)
    grid.add_column("desc", ratio=1)
    grid.add_column("shortcut", width=4, justify="right")

    for i, cmd in enumerate(matches):
        is_sel = i == selected_idx
        selector = f"[bold {t.sel_left}]▸[/]" if is_sel else f"[{t.text_faint}] [/]"
        name = (
            f"[bold {t.accent} on {t.sel_bg}] {cmd.name} [/]"
            if is_sel
            else f"[{t.accent}]{cmd.name}[/]"
        )
        desc = (
            f"[{t.text_muted} on {t.sel_bg}] {cmd.description} [/]"
            if is_sel
            else f"[{t.text_muted}]{cmd.description}[/]"
        )
        sc = SHORTCUT_LABELS.get(cmd.name, cmd.shortcut or "")
        if sc:
            shortcut = (
                f"[{t.kbd_text} on {t.kbd_bg}] {sc} [/]"
                if is_sel
                else f"[{t.kbd_text} on {t.kbd_bg}] {sc} [/]"
            )
        else:
            shortcut = ""
        grid.add_row(selector, name, desc, shortcut)

    sub_prompt = f"\n[{t.text_muted}]  / [/][{t.accent}]{query}[/][{t.text_faint}]_[/]"

    body = Table.grid(expand=True, padding=(0, 0))
    body.add_column()
    body.add_row(grid)
    body.add_row(sub_prompt)

    return Panel(
        body,
        title=f"[{t.accent}]COMMANDS[/]",
        title_align="left",
        border_style=t.border,
        box=box.HORIZONTALS,
        padding=(0, 1),
    )


def show_palette(
    session: PromptSession,
    registry: list[Command],
) -> Optional[str]:
    """Render the command palette, return the selected command name or None."""
    t = get_active_theme()
    console = get_console()
    query = ""
    selected_idx = 0
    matches = list(registry)

    palette_kb = KeyBindings()

    @palette_kb.add("escape")
    def _esc(event) -> None:
        event.app.exit(result=None)

    @palette_kb.add("enter")
    def _enter(event) -> None:
        if matches:
            event.app.exit(result=matches[selected_idx].name)
        else:
            event.app.exit(result=None)

    @palette_kb.add("up")
    def _up(event) -> None:
        nonlocal selected_idx
        if matches:
            selected_idx = (selected_idx - 1) % len(matches)

    @palette_kb.add("down")
    def _down(event) -> None:
        nonlocal selected_idx
        if matches:
            selected_idx = (selected_idx + 1) % len(matches)

    def _on_text_change(buf) -> None:
        nonlocal query, matches, selected_idx
        query = buf.text
        matches = _filter_commands(registry, query)
        selected_idx = 0

    sub_session: PromptSession[str | None] = PromptSession(
        history=InMemoryHistory(),
        key_bindings=palette_kb,
        prompt=HTML(f"<span style='color:{t.text_muted}'>/</span> "),
    )

    console.print()
    with Live(
        _render_palette(console, matches, selected_idx, ""),
        console=console,
        refresh_per_second=30,
        transient=False,
        screen=False,
    ) as live:
        try:
            result = sub_session.prompt(
                "",
                on_text_changed=_on_text_change,
            )
        except (KeyboardInterrupt, EOFError):
            return None
        if result is None:
            return None
        query = result
        matches = _filter_commands(registry, query)
        if not matches:
            return None
        return matches[0].name
