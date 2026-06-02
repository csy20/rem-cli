"""Stage-based pipeline runner with durable state for resume/retry."""

from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable


def _utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


@dataclass
class PipelineStage:
    name: str
    run: Callable[[], None]


class PipelineState:
    def __init__(self, state_path: Path, run_id: str):
        self.state_path = state_path
        self.run_id = run_id
        self.payload = self._load_or_init()

    def _load_or_init(self) -> dict:
        if self.state_path.exists():
            with self.state_path.open("r", encoding="utf-8") as handle:
                return json.load(handle)
        return {
            "run_id": self.run_id,
            "created_at": _utc_now(),
            "updated_at": _utc_now(),
            "stages": {},
        }

    def _write(self) -> None:
        self.state_path.parent.mkdir(parents=True, exist_ok=True)
        self.payload["updated_at"] = _utc_now()
        temp_path = self.state_path.with_suffix(".tmp")
        temp_path.write_text(json.dumps(self.payload, indent=2), encoding="utf-8")
        temp_path.replace(self.state_path)

    def get_stage(self, stage_name: str) -> dict:
        return self.payload.setdefault("stages", {}).setdefault(stage_name, {})

    def mark_started(self, stage_name: str) -> None:
        stage = self.get_stage(stage_name)
        stage["status"] = "running"
        stage["attempts"] = int(stage.get("attempts", 0)) + 1
        stage["started_at"] = _utc_now()
        self._write()

    def mark_completed(self, stage_name: str) -> None:
        stage = self.get_stage(stage_name)
        stage["status"] = "completed"
        stage["completed_at"] = _utc_now()
        stage.pop("last_error", None)
        self._write()

    def mark_failed(self, stage_name: str, error: str) -> None:
        stage = self.get_stage(stage_name)
        stage["status"] = "failed"
        stage["last_error"] = error
        stage["failed_at"] = _utc_now()
        self._write()

    def is_completed(self, stage_name: str) -> bool:
        return self.get_stage(stage_name).get("status") == "completed"


class PipelineRunner:
    def __init__(
        self,
        run_id: str,
        state_path: Path,
        max_retries: int = 1,
        force_rerun: bool = False,
    ):
        self.run_id = run_id
        self.max_retries = max(1, int(max_retries))
        self.force_rerun = force_rerun
        self.state = PipelineState(state_path, run_id)

    def run_stages(self, stages: list[PipelineStage], log) -> None:
        for stage in stages:
            if self.state.is_completed(stage.name) and not self.force_rerun:
                log.info(
                    "pipeline_stage_skipped",
                    stage=stage.name,
                    reason="already_completed",
                )
                continue

            last_error: Exception | None = None
            for attempt in range(1, self.max_retries + 1):
                self.state.mark_started(stage.name)
                log.info(
                    "pipeline_stage_start",
                    stage=stage.name,
                    attempt=attempt,
                    max_retries=self.max_retries,
                )
                try:
                    stage.run()
                    self.state.mark_completed(stage.name)
                    log.info(
                        "pipeline_stage_complete", stage=stage.name, attempt=attempt
                    )
                    last_error = None
                    break
                except Exception as exc:  # pragma: no cover - exercised by tests
                    last_error = exc
                    self.state.mark_failed(stage.name, str(exc))
                    log.warning(
                        "pipeline_stage_failed",
                        stage=stage.name,
                        attempt=attempt,
                        error=str(exc),
                    )

            if last_error is not None:
                raise last_error
