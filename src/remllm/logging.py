"""Structured logging with optional structlog support."""

import logging
import os
import sys
from pathlib import Path
from typing import Any

try:
    import structlog  # type: ignore[import-not-found]
except ModuleNotFoundError:  # pragma: no cover - exercised by environment/tests
    structlog = None

_initialized = False
_log_dir: Path | None = None


class _FallbackBoundLogger:
    """Minimal structlog-like adapter backed by stdlib logging."""

    def __init__(
        self, logger: logging.Logger, context: dict[str, Any] | None = None
    ) -> None:
        self._logger = logger
        self._context = context or {}

    def bind(self, **context: Any) -> "_FallbackBoundLogger":
        merged = dict(self._context)
        merged.update(context)
        return _FallbackBoundLogger(self._logger, merged)

    def _fmt(self, event: str, **kwargs: Any) -> str:
        merged = dict(self._context)
        merged.update(kwargs)
        if not merged:
            return event
        fields = " ".join(f"{k}={v!r}" for k, v in sorted(merged.items()))
        return f"{event} {fields}"

    def debug(self, event: str, **kwargs: Any) -> None:
        self._logger.debug(self._fmt(event, **kwargs))

    def info(self, event: str, **kwargs: Any) -> None:
        self._logger.info(self._fmt(event, **kwargs))

    def warning(self, event: str, **kwargs: Any) -> None:
        self._logger.warning(self._fmt(event, **kwargs))

    def error(self, event: str, **kwargs: Any) -> None:
        self._logger.error(self._fmt(event, **kwargs))


def init_logging() -> None:
    """Initialize structured logging for human-readable console output."""
    global _initialized

    if _initialized:
        return
    _initialized = True

    if structlog is None:
        logging.basicConfig(
            level=os.environ.get("REMLLM_LOG_LEVEL", "INFO"),
            stream=sys.stderr,
            format="%(asctime)s %(levelname)s %(message)s",
        )
        return

    structlog.configure(
        processors=[
            structlog.stdlib.add_log_level,
            structlog.stdlib.PositionalArgumentsFormatter(),
            structlog.processors.TimeStamper(fmt="%H:%M:%S", utc=True),
            structlog.processors.StackInfoRenderer(),
            structlog.processors.format_exc_info,
            structlog.processors.UnicodeDecoder(),
            structlog.dev.ConsoleRenderer(pad_event_to=0),
        ],
        context_class=dict,
        logger_factory=structlog.PrintLoggerFactory(sys.stderr),
        wrapper_class=structlog.stdlib.BoundLogger,
        cache_logger_on_first_use=True,
    )


def get_logger(**context: Any):
    """Get a bound structured logger.

    Usage:
        log = get_logger(run_id="exp-001", phase="train")
        log.info("Starting training", epochs=3, lr=1.2e-4)
    """
    if not _initialized:
        init_logging()
    if structlog is None:
        base = logging.getLogger("remllm")
        return _FallbackBoundLogger(base, context)
    return structlog.get_logger().bind(**context)


def set_log_dir(path: str | Path) -> None:
    """Set or change the log directory."""
    global _log_dir
    _log_dir = Path(path)
    _log_dir.mkdir(parents=True, exist_ok=True)


def get_log_dir() -> Path | None:
    """Return the current log directory."""
    return _log_dir
