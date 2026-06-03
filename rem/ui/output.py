"""Streaming output: spinner -> Live -> final Panel with Syntax/Markdown rendering."""

from __future__ import annotations

import re
from typing import Iterable

from rich import box
from rich.console import Console, Group, RenderableType
from rich.live import Live
from rich.markdown import Markdown
from rich.panel import Panel
from rich.spinner import Spinner
from rich.syntax import Syntax
from rich.text import Text

from rem.ui.theme import get_active_theme, get_console


_CODE_BLOCK_RE = re.compile(r"```(\w*)\n?(.*?)```", re.DOTALL)


def _syntax_theme() -> str:
    """Return the Syntax theme name based on the active theme."""
    name = get_active_theme().name
    if name == "PAPER":
        return "friendly"
    return "monokai"


def _render_content(text: str) -> RenderableType:
    """Render accumulated stream text, splitting code blocks into Syntax panels."""
    if not text:
        return Text("")
    theme = _syntax_theme()
    parts: list[RenderableType] = []
    pos = 0
    for match in _CODE_BLOCK_RE.finditer(text):
        if match.start() > pos:
            parts.append(Markdown(text[pos : match.start()], code_theme=theme))
        lang = (match.group(1) or "python").strip() or "python"
        code = match.group(2).rstrip("\n")
        parts.append(
            Syntax(
                code,
                lang,
                theme=theme,
                word_wrap=True,
                background_color="default",
            )
        )
        pos = match.end()
    if pos < len(text):
        parts.append(Markdown(text[pos:], code_theme=theme))
    if not parts:
        return Markdown(text, code_theme=theme)
    if len(parts) == 1:
        return parts[0]
    return Group(*parts)


def stream_response(generator: Iterable[str], model_name: str) -> str:
    """Stream tokens from `generator`, render with Spinner -> Live -> final Panel."""
    t = get_active_theme()
    console = get_console()
    full_text = ""
    spinner = Spinner("dots", text="thinking...", style=t.accent_dim)

    def _display() -> RenderableType:
        if not full_text:
            return spinner
        return _render_content(full_text)

    with Live(spinner, console=console, refresh_per_second=20, transient=True) as live:
        try:
            for token in generator:
                if not isinstance(token, str):
                    token = str(token)
                full_text += token
                live.update(_display())
        except (KeyboardInterrupt, EOFError):
            console.print(f"[{t.sys_color}](stream interrupted)[/]")

    final = Panel(
        _render_content(full_text),
        border_style=t.border,
        box=box.HORIZONTALS,
        title=f"[{t.accent_dim}]{model_name}[/]",
        title_align="left",
        padding=(0, 1),
    )
    console.print(final)
    return full_text
