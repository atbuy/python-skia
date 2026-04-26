"""Tiny CLI client used by compositor-bound hotkeys.

Bind a key in your Wayland compositor (GNOME/KDE/Hyprland/Sway) to:

    python -m skia.save

It connects to the running `python -m skia.main` supervisor over a Unix
socket and triggers a save of the last clip.
"""

from __future__ import annotations

import argparse
import sys

from skia.control_socket import default_socket_path, send_command


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--socket",
        default=None,
        help="path to the Skia control socket (default: $XDG_RUNTIME_DIR/skia.sock)",
    )
    args = parser.parse_args()

    path = args.socket
    try:
        send_command("save", path=path)
    except FileNotFoundError as error:
        print(error, file=sys.stderr)
        return 1
    except OSError as error:
        print(f"failed to talk to Skia: {error}", file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
