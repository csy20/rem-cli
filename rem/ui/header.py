"""Header panel: startup banner with model, mode pill, and hint text."""

from __future__ import annotations

from rich import box
from rich.console import Console
from rich.panel import Panel
from rich.table import Table

from rem.ui.theme import get_active_theme, get_console

VERSION = "0.1.0"
RAM_WARNING: str | None = None


def set_ram_warning(message: str | None) -> None:
    """Set a system-level RAM warning string printed above the header."""
    global RAM_WARNING
    RAM_WARNING = message


def _build_header_table(theme_name: str, model: str, mode: str) -> Table:
    t = get_active_theme()
    main = (
        f"[bold {t.accent}]REM v{VERSION}[/]"
        f"  [{t.text_faint}]·[/]  "
        f"[{t.text_muted}]model[/] "
        f"[{t.accent_dim}]{model}[/]"
        f"  [{t.text_faint}]·[/]  "
        f"[bold {t.pill_text} on {t.pill_bg}] [ {mode} ][/]"
    )
    hint = f"[{t.text_faint}]/  for commands   ↑↓  history[/]"
    grid = Table.grid(expand=True)
    grid.add_column("main", justify="left")
    grid.add_column("hint", justify="right")
    grid.add_row(main, hint)
    return grid


def render_header(model: str, mode: str, console: Console | None = None) -> None:
    """Print the startup header panel using the active theme."""
    t = get_active_theme()
    con = console or get_console()
    if RAM_WARNING:
        con.print(f"[{t.sys_color}]{RAM_WARNING}[/]")
    grid = _build_header_table(t.name, model, mode)
    panel = Panel(
        grid,
        border_style=t.border,
        box=box.HORIZONTALS,
        padding=(0, 1),
        expand=False,
    )
    con.print(panel)
