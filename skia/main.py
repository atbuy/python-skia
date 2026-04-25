from datetime import datetime
from pathlib import Path
import shutil
import subprocess
import time
from typing import Any

from pynput import keyboard

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
        print("Starting Skia...")
        self.recorder.start_process()
        self.recorder.start_recording(
            clip_seconds=self.config.clip_seconds,
            segment_seconds=self.config.segment_seconds,
            backend=self.config.backend,
        )

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

    def save_clip(self) -> None:
        filename = datetime.now().strftime("%Y%m%d-%H%M%S")
        output = self.output_dir.joinpath(f"{filename}.mp4")
        self.recorder.save_last(seconds=self.config.clip_seconds, output=output)

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
    skia = SkiaApp()
    skia.start()


if __name__ == "__main__":
    main()
