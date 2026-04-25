import argparse
from datetime import datetime
from pathlib import Path
import shutil
import subprocess
import time
from typing import Any

from skia.config import SkiaConfig, load_config
from skia.recorder_client import RecorderClient


class SkiaApp:
    def __init__(self, config: SkiaConfig | None = None):
        self.root = Path(__file__).resolve().parent.parent
        self.config = config or load_config(root=self.root)
        self.output_dir = self.config.output_dir
        self.output_dir.mkdir(exist_ok=True)
        self.recorder = RecorderClient(on_event=self._on_event, on_log=self._on_log)

    def start(self) -> None:
        from pynput import keyboard

        print("Starting Skia...")
        self.start_recorder()

        with keyboard.GlobalHotKeys({self.config.hotkey: self.save_clip}) as hotkeys:
            try:
                while True:
                    time.sleep(0.25)
            except KeyboardInterrupt:
                pass
            finally:
                hotkeys.stop()
                self.recorder.close()
                print("Stopped Skia.")

    def smoke(self, *, warmup_seconds: float) -> int:
        print("Starting Skia smoke test...")
        self.start_recorder()
        try:
            if not self._wait_for_event("recording_started", timeout=30):
                print("Recorder did not start.")
                return 1

            time.sleep(warmup_seconds)
            self.save_clip()
            event = self._wait_for_any_event({"clip_saved", "error"}, timeout=30)
            if event is None:
                print("Timed out waiting for clip result.")
                return 1

            return 0 if event.get("event") == "clip_saved" else 1
        finally:
            self.recorder.close()
            print("Stopped Skia smoke test.")

    def check(self) -> int:
        self.recorder.start_process()
        try:
            self.recorder.check()
            event = self._wait_for_any_event({"check", "error"}, timeout=10)
            if event is None:
                print("Timed out waiting for recorder check.")
                return 1

            if event.get("event") == "check":
                print(f"Recorder check: {event.get('runtime')}")
                return 0

            return 1
        finally:
            self.recorder.close()

    def start_recorder(self) -> None:
        self.recorder.start_process()
        self.recorder.start_recording(
            clip_seconds=self.config.clip_seconds,
            segment_seconds=self.config.segment_seconds,
            backend=self.config.backend,
            fps=self.config.fps,
            cache_dir=self.config.cache_dir,
            video_input=self.config.video_input,
            audio_input=self.config.audio_input,
        )

    def save_clip(self) -> None:
        filename = datetime.now().strftime("%Y%m%d-%H%M%S")
        output = self.output_dir.joinpath(f"{filename}.mp4")
        self.recorder.save_last(seconds=self.config.clip_seconds, output=output)

    def _wait_for_event(self, event_type: str, *, timeout: float) -> dict[str, Any] | None:
        return self._wait_for_any_event({event_type}, timeout=timeout)

    def _wait_for_any_event(
        self,
        event_types: set[str],
        *,
        timeout: float,
    ) -> dict[str, Any] | None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            event = self.recorder.wait_for_event(timeout=0.25)
            if event is None:
                continue
            if event.get("event") in event_types:
                return event
        return None

    def _on_event(self, event: dict[str, Any]) -> None:
        event_type = event.get("event")
        if event_type == "ready":
            print(f"Recorder ready: {event.get('version')}")
        elif event_type == "recording_started":
            print(f"Recording with backend: {event.get('backend')}")
        elif event_type == "clip_saved":
            path = str(event.get("path"))
            print(f"Clip saved: {path}")
            self._notify("Skia", f"Clip saved: {path}")
        elif event_type == "error":
            message = str(event.get("message", "unknown recorder error"))
            print(f"Recorder error: {message}")
            self._notify("Skia", message)
        else:
            print(f"Recorder event: {event}")

    def _on_log(self, line: str) -> None:
        if line:
            print(f"recorder: {line}")

    def _notify(self, title: str, message: str) -> None:
        if shutil.which("notify-send") is None:
            return

        subprocess.run(
            ["notify-send", title, message],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true", help="print recorder runtime checks")
    parser.add_argument("--smoke", action="store_true", help="run one start/save/stop smoke test")
    parser.add_argument(
        "--smoke-warmup",
        type=float,
        default=5.0,
        help="seconds to record before saving in smoke mode",
    )
    args = parser.parse_args()

    skia = SkiaApp()
    if args.check:
        raise SystemExit(skia.check())

    if args.smoke:
        raise SystemExit(skia.smoke(warmup_seconds=args.smoke_warmup))

    skia.start()


if __name__ == "__main__":
    main()
