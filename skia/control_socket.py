"""Unix socket control plane for the running Skia supervisor.

Wayland blocks global keyboard grabs, so we cannot use pynput-style hotkeys.
Instead, the supervisor listens on a Unix socket and the user binds a
compositor-level hotkey to a tiny client (`python -m skia.save`) that opens
the socket and writes a single command line.
"""

from __future__ import annotations

import os
import socket
import threading
from pathlib import Path
from typing import Callable


CommandHandler = Callable[[str], None]


def default_socket_path() -> Path:
    runtime_dir = os.environ.get("XDG_RUNTIME_DIR")
    if runtime_dir:
        return Path(runtime_dir) / "skia.sock"
    return Path(f"/tmp/skia-{os.getuid()}.sock")


class ControlSocketServer:
    def __init__(self, path: Path, on_command: CommandHandler):
        self.path = path
        self.on_command = on_command
        self._server: socket.socket | None = None
        self._thread: threading.Thread | None = None
        self._stop = threading.Event()

    def start(self) -> None:
        if self._server is not None:
            return

        if self.path.exists():
            try:
                self.path.unlink()
            except OSError:
                pass

        self.path.parent.mkdir(parents=True, exist_ok=True)
        server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        server.bind(str(self.path))
        os.chmod(self.path, 0o600)
        server.listen(4)
        server.settimeout(0.5)
        self._server = server

        self._thread = threading.Thread(target=self._accept_loop, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        server = self._server
        self._server = None
        if server is not None:
            try:
                server.close()
            except OSError:
                pass
        thread = self._thread
        self._thread = None
        if thread is not None:
            thread.join(timeout=1.0)
        try:
            self.path.unlink()
        except FileNotFoundError:
            pass
        except OSError:
            pass

    def _accept_loop(self) -> None:
        while not self._stop.is_set():
            server = self._server
            if server is None:
                return
            try:
                client, _ = server.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            with client:
                client.settimeout(1.0)
                try:
                    data = client.recv(64)
                except (socket.timeout, OSError):
                    continue
            command = data.decode("utf-8", errors="replace").strip()
            if command:
                try:
                    self.on_command(command)
                except Exception as error:  # noqa: BLE001
                    print(f"control socket handler error: {error}")


def send_command(command: str, *, path: Path | None = None, timeout: float = 2.0) -> None:
    target = path or default_socket_path()
    if not target.exists():
        raise FileNotFoundError(
            f"Skia control socket not found at {target}; is `python -m skia.main` running?"
        )

    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.settimeout(timeout)
        client.connect(str(target))
        client.sendall((command.strip() + "\n").encode("utf-8"))
