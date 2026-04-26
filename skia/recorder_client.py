import json
import subprocess
import sys
from pathlib import Path
from queue import Empty, Queue
from threading import Thread
from typing import Any, Callable


EventHandler = Callable[[dict[str, Any]], None]
LogHandler = Callable[[str], None]


class RecorderClient:
    def __init__(
        self,
        command: list[str] | None = None,
        *,
        cwd: Path | None = None,
        on_event: EventHandler | None = None,
        on_log: LogHandler | None = None,
    ):
        self.cwd = cwd or Path(__file__).resolve().parent.parent
        self.command = command or self._default_command()
        self.on_event = on_event
        self.on_log = on_log
        self._process: subprocess.Popen[str] | None = None
        self._events: Queue[dict[str, Any]] = Queue()
        self._next_id = 0
        self._closing = False

    def start_process(self) -> None:
        if self._process is not None and self._process.poll() is None:
            return

        self._closing = False
        self._process = subprocess.Popen(
            self.command,
            cwd=self.cwd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )

        Thread(target=self._read_stdout, daemon=True).start()
        Thread(target=self._read_stderr, daemon=True).start()
        Thread(target=self._monitor_process, daemon=True).start()

    def start_recording(
        self,
        *,
        clip_seconds: int = 30,
        segment_seconds: int = 2,
        backend: str = "auto",
        fps: int = 60,
        cache_dir: Path | None = None,
        video_input: str | None = None,
        audio_input: str | None = None,
        gstreamer: dict[str, Any] | None = None,
    ) -> str:
        config: dict[str, Any] = {
            "clip_seconds": clip_seconds,
            "segment_seconds": segment_seconds,
            "backend": backend,
            "fps": fps,
        }
        if cache_dir is not None:
            config["cache_dir"] = str(cache_dir)
        if video_input is not None:
            config["video_input"] = video_input
        if audio_input is not None:
            config["audio_input"] = audio_input
        if gstreamer:
            config["gstreamer"] = gstreamer

        return self.send(
            "start",
            config=config,
        )

    def save_last(self, *, seconds: int, output: Path) -> str:
        return self.send("save_last", seconds=seconds, output=str(output))

    def status(self) -> str:
        return self.send("status")

    def check(self) -> str:
        return self.send("check")

    def stop_recording(self) -> str:
        return self.send("stop")

    def send(self, command: str, **payload: Any) -> str:
        process = self._running_process()
        if process.stdin is None:
            raise RuntimeError("recorder stdin is unavailable")

        command_id = self._command_id()
        message = {"id": command_id, "cmd": command, **payload}
        process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
        process.stdin.flush()
        return command_id

    def wait_for_event(self, timeout: float | None = None) -> dict[str, Any] | None:
        try:
            return self._events.get(timeout=timeout)
        except Empty:
            return None

    def close(self) -> None:
        process = self._process
        if process is None:
            return

        self._closing = True
        if process.poll() is None:
            try:
                self.stop_recording()
            except RuntimeError:
                pass

            if process.stdin is not None:
                process.stdin.close()

            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                process.terminate()
                try:
                    process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=2)

        self._close_pipes(process)

        self._process = None

    def _default_command(self) -> list[str]:
        binary = self.cwd.joinpath("target", "debug", "skia-recorder")
        if sys.platform == "win32":
            binary = binary.with_suffix(".exe")

        if binary.exists():
            return [str(binary)]

        return ["cargo", "run", "-q", "-p", "skia-recorder"]

    def _running_process(self) -> subprocess.Popen[str]:
        process = self._process
        if process is None or process.poll() is not None:
            raise RuntimeError("recorder process is not running")
        return process

    def _command_id(self) -> str:
        self._next_id += 1
        return str(self._next_id)

    def _read_stdout(self) -> None:
        process = self._process
        if process is None or process.stdout is None:
            return

        for line in process.stdout:
            line = line.strip()
            if not line:
                continue

            try:
                event = json.loads(line)
            except json.JSONDecodeError as error:
                event = {
                    "event": "error",
                    "id": None,
                    "code": "invalid_event",
                    "message": str(error),
                }

            self._events.put(event)
            if self.on_event is not None:
                self.on_event(event)

    def _read_stderr(self) -> None:
        process = self._process
        if process is None or process.stderr is None:
            return

        for line in process.stderr:
            line = line.rstrip()
            if self.on_log is not None:
                self.on_log(line)

    def _monitor_process(self) -> None:
        process = self._process
        if process is None:
            return

        return_code = process.wait()
        if self._closing:
            return

        event = {
            "event": "error",
            "id": None,
            "code": "daemon_exited",
            "message": f"recorder daemon exited with code {return_code}",
        }
        self._events.put(event)
        if self.on_event is not None:
            self.on_event(event)

    def _close_pipes(self, process: subprocess.Popen[str]) -> None:
        for pipe in (process.stdin, process.stdout, process.stderr):
            if pipe is not None and not pipe.closed:
                pipe.close()
