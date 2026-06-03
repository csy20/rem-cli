"""Prompt line: prompt_toolkit session with / palette binding and mode-aware prefix."""

from __future__ import annotations

from typing import Optional

from prompt_toolkit import PromptSession
from prompt_toolkit.formatted_text import HTML
from prompt_toolkit.history import InMemoryHistory
from prompt_toolkit.key_binding import KeyBindings

from rem import config
from rem.commands.registry import REGISTRY
from rem.ui.palette import show_palette
from rem.ui.theme import get_active_theme


_session: Optional[PromptSession] = None


def _prompt_html() -> HTML:
    """Return the colored prompt prefix for the current mode."""
    t = get_active_theme()
    mode = str(config.get("mode") or "CHAT")
    if mode == "CHAT":
        model = str(config.get("model") or "rem")
        return HTML(
            f"<span style='color:{t.accent_dim}'>{model}</span>"
            f"<span style='color:{t.text_faint}'> ›</span> "
        )
    return HTML(
        f"<span style='color:{t.accent}'>{{}}</span>"
        f"<span style='color:{t.text_faint}'> ›</span> "
    )


def _build_session() -> PromptSession:
    """Build the singleton PromptSession with palette + control key bindings."""
    kb = KeyBindings()

    @kb.add("/")
    def _slash(event) -> None:
        buf = event.current_buffer
        if buf.text == "" and buf.cursor_position == 0:
            result = show_palette(_session, REGISTRY)
            if result:
                buf.insert_text(result)
        else:
            buf.insert_text("/")

    @kb.add("c-c")
    def _ctrl_c(event) -> None:
        event.app.exit(result="")

    @kb.add("c-d")
    def _ctrl_d(event) -> None:
        event.app.exit(result="/exit")

    return PromptSession(
        history=InMemoryHistory(),
        key_bindings=kb,
    )


def get_input(mode: str) -> str:
    """Read one user input line; `/` at empty buffer opens the command palette."""
    del mode
    global _session
    if _session is None:
        _session = _build_session()
    try:
        result = _session.prompt(prompt=_prompt_html)
    except (KeyboardInterrupt, EOFError):
        return ""
    return result or ""


def reset_session() -> None:
    """Discard the cached PromptSession; useful after theme/setting changes."""
    global _session
    _session = None
