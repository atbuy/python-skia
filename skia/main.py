import argparse
from datetime import datetime
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time
from typing import Any

from skia.config import SkiaConfig, load_config
from skia.control_socket import ControlSocketServer, default_socket_path
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
        self.start_recorder()

        socket_path = default_socket_path()
        server = ControlSocketServer(socket_path, on_command=self._handle_socket_command)
        server.start()
        print(f"Control socket: {socket_path}")
        print(f"Bind a compositor hotkey to: python -m skia.save")

        try:
            while True:
                time.sleep(0.25)
        except KeyboardInterrupt:
            pass
        finally:
            server.stop()
            self.recorder.close()
            print("Stopped Skia.")

    def _handle_socket_command(self, command: str) -> None:
        if command == "save":
            self.save_clip()
        else:
            print(f"unknown control command: {command!r}")

    def smoke(self, *, warmup_seconds: float) -> int:
        print("Starting Skia smoke test...")
        self.start_recorder()
        try:
            start_event = self._wait_for_any_event({"recording_started", "error"}, timeout=30)
            if start_event is None:
                print("Recorder did not start.")
                return 1
            if start_event.get("event") == "error":
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
        audio_input = self.config.audio_input
        if audio_input is None:
            audio_input = _detect_default_audio_monitor()
            if audio_input is not None:
                print(f"Auto-detected audio source: {audio_input}")

        gstreamer_overrides: dict[str, Any] = {}
        gst = self.config.gstreamer
        if gst.bitrate_kbps is not None:
            gstreamer_overrides["bitrate_kbps"] = gst.bitrate_kbps
        if gst.quantizer is not None:
            gstreamer_overrides["quantizer"] = gst.quantizer
        if gst.x264_preset is not None:
            gstreamer_overrides["x264_preset"] = gst.x264_preset
        if gst.audio_bitrate_bps is not None:
            gstreamer_overrides["audio_bitrate_bps"] = gst.audio_bitrate_bps

        self.recorder.start_process()
        self.recorder.start_recording(
            clip_seconds=self.config.clip_seconds,
            segment_seconds=self.config.segment_seconds,
            backend=self.config.backend,
            fps=self.config.fps,
            cache_dir=self.config.cache_dir,
            video_input=self.config.video_input,
            audio_input=audio_input,
            gstreamer=gstreamer_overrides or None,
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
            path = Path(str(event.get("path")))
            print(f"Clip saved: {path}")
            threading.Thread(
                target=self._notify_clip_saved,
                args=(path,),
                daemon=True,
            ).start()
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
            ["notify-send", "--app-name=Skia", title, message],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

    def _notify_clip_saved(self, path: Path) -> None:
        if shutil.which("notify-send") is None:
            return

        thumbnail = _make_thumbnail(path)
        try:
            args = ["notify-send", "--app-name=Skia"]
            if thumbnail is not None:
                args.append(f"--icon={thumbnail}")

            if _notify_send_supports_actions():
                args += ["--action=open=Open folder", "--wait"]
                args += ["Skia", f"Clip saved: {path}"]
                try:
                    result = subprocess.run(
                        args,
                        capture_output=True,
                        text=True,
                        timeout=120,
                    )
                except subprocess.TimeoutExpired:
                    return
                if result.returncode == 0 and result.stdout.strip() == "open":
                    _open_folder(path.parent)
            else:
                args += ["Skia", f"Clip saved: {path}"]
                subprocess.run(
                    args,
                    check=False,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
        finally:
            if thumbnail is not None:
                try:
                    thumbnail.unlink()
                except OSError:
                    pass


_action_support: bool | None = None
_action_support_lock = threading.Lock()


def _notify_send_supports_actions() -> bool:
    global _action_support
    with _action_support_lock:
        if _action_support is None:
            try:
                output = subprocess.run(
                    ["notify-send", "--help"],
                    capture_output=True,
                    text=True,
                    timeout=5,
                )
                combined = (output.stdout or "") + (output.stderr or "")
                _action_support = "--action" in combined
            except (OSError, subprocess.SubprocessError):
                _action_support = False
        return _action_support


def _make_thumbnail(clip: Path) -> Path | None:
    if shutil.which("ffmpeg") is None:
        return None

    fd, tmp_path = tempfile.mkstemp(prefix="skia-thumb-", suffix=".png")
    os.close(fd)
    tmp = Path(tmp_path)
    cmd = [
        "ffmpeg",
        "-y",
        "-loglevel",
        "error",
        "-ss",
        "1",
        "-i",
        str(clip),
        "-frames:v",
        "1",
        "-vf",
        "scale=320:-1",
        str(tmp),
    ]
    try:
        result = subprocess.run(cmd, capture_output=True, timeout=15)
    except (OSError, subprocess.SubprocessError):
        try:
            tmp.unlink()
        except OSError:
            pass
        return None

    if result.returncode != 0 or not tmp.exists() or tmp.stat().st_size == 0:
        try:
            tmp.unlink()
        except OSError:
            pass
        return None
    return tmp


def _detect_default_audio_monitor() -> str | None:
    if shutil.which("pactl") is None:
        return None
    try:
        result = subprocess.run(
            ["pactl", "get-default-sink"],
            capture_output=True,
            text=True,
            timeout=5,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if result.returncode != 0:
        return None
    sink = result.stdout.strip()
    if not sink:
        return None
    return f"{sink}.monitor"


def _open_folder(folder: Path) -> None:
    opener = shutil.which("xdg-open") or shutil.which("open")
    if opener is None:
        return
    subprocess.Popen(
        [opener, str(folder)],
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
