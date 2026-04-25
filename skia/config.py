import os
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping


DEFAULT_CLIP_SECONDS = 30
DEFAULT_SEGMENT_SECONDS = 2
DEFAULT_BACKEND = "auto"
DEFAULT_HOTKEY = "<ctrl>+."
DEFAULT_OUTPUT_DIR = "out"
DEFAULT_CACHE_DIR = ".cache/skia"
DEFAULT_FPS = 60


@dataclass(frozen=True)
class SkiaConfig:
    clip_seconds: int
    segment_seconds: int
    backend: str
    fps: int
    cache_dir: Path
    video_input: str | None
    audio_input: str | None
    hotkey: str
    output_dir: Path


def load_config(
    path: Path | None = None,
    *,
    root: Path | None = None,
    env: Mapping[str, str] = os.environ,
) -> SkiaConfig:
    root = root or Path(__file__).resolve().parent.parent
    config_path = _config_path(path, root=root, env=env)
    data = _read_toml(config_path) if config_path is not None else {}

    recording = data.get("recording", {})
    app = data.get("app", {})

    clip_seconds = _positive_int(
        recording.get("clip_seconds", DEFAULT_CLIP_SECONDS),
        "recording.clip_seconds",
    )
    segment_seconds = _positive_int(
        recording.get("segment_seconds", DEFAULT_SEGMENT_SECONDS),
        "recording.segment_seconds",
    )
    backend = str(recording.get("backend", DEFAULT_BACKEND))
    fps = _positive_int(recording.get("fps", DEFAULT_FPS), "recording.fps")
    cache_dir = _resolve_path(root, recording.get("cache_dir", DEFAULT_CACHE_DIR))
    video_input = _optional_str(recording.get("video_input"))
    audio_input = _optional_str(recording.get("audio_input"))
    hotkey = str(app.get("hotkey", DEFAULT_HOTKEY))
    output_dir = _resolve_path(root, app.get("output_dir", DEFAULT_OUTPUT_DIR))

    return SkiaConfig(
        clip_seconds=clip_seconds,
        segment_seconds=segment_seconds,
        backend=backend,
        fps=fps,
        cache_dir=cache_dir,
        video_input=video_input,
        audio_input=audio_input,
        hotkey=hotkey,
        output_dir=output_dir,
    )


def _config_path(
    path: Path | None,
    *,
    root: Path,
    env: Mapping[str, str],
) -> Path | None:
    if path is not None:
        return path

    if env.get("SKIA_CONFIG"):
        return Path(env["SKIA_CONFIG"])

    default_path = root.joinpath("skia.toml")
    if default_path.exists():
        return default_path

    return None


def _read_toml(path: Path) -> dict:
    with path.open("rb") as file:
        return tomllib.load(file)


def _positive_int(value: object, key: str) -> int:
    if not isinstance(value, int) or value <= 0:
        raise ValueError(f"{key} must be a positive integer")
    return value


def _resolve_path(root: Path, value: object) -> Path:
    path = Path(str(value)).expanduser()
    if path.is_absolute():
        return path
    return root.joinpath(path)


def _optional_str(value: object) -> str | None:
    if value is None:
        return None
    return str(value)
