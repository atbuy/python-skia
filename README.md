# Skia

A clip tool that works like ShadowPlay: keep recording in the background, then press a hotkey to save the last few seconds.

Phase 1 uses Python as the control app and a Rust recorder daemon for capture, segment storage, and export. See `docs/architecture.md`.

## Requirements

- Python from the active pyenv environment
- Rust/Cargo
- FFmpeg
- Linux Wayland: `pipewire`, `xdg-desktop-portal`, and a desktop portal backend such as `xdg-desktop-portal-gtk`, `xdg-desktop-portal-gnome`, or `xdg-desktop-portal-kde`
- Linux X11: FFmpeg with `x11grab`

Check FFmpeg capture devices:

```bash
ffmpeg -hide_banner -devices
```

## Configuration

Copy `skia.example.toml` to `skia.toml` and edit values as needed. Defaults are used when no config file exists.

Important fields:

- `recording.backend`: `auto`, `linux-wayland-ffmpeg`, `linux-x11-ffmpeg`, `windows-ffmpeg`, or `macos-ffmpeg`
- `recording.clip_seconds`: clip length saved by the hotkey
- `recording.segment_seconds`: internal segment duration
- `recording.cache_dir`: temporary segment cache
- `recording.video_input`: optional FFmpeg video input override
- `recording.audio_input`: optional FFmpeg audio input override
- `app.hotkey`: default `<ctrl>+.`
- `app.output_dir`: final clip output directory

## Run

Build and test the Rust daemon:

```bash
cargo test -p skia-recorder
```

Run the local FFmpeg smoke test:

```bash
cargo test -p skia-recorder --test ffmpeg_smoke -- --ignored
```

Run platform capture smoke tests on matching systems:

```bash
cargo test -p skia-recorder --test backend_smoke x11_ffmpeg_capture_smoke -- --ignored
cargo test -p skia-recorder --test backend_smoke windows_gdigrab_capture_smoke -- --ignored
cargo test -p skia-recorder --test backend_smoke macos_avfoundation_capture_smoke -- --ignored
```

Run the app:

```bash
python -m skia.main
```

On Wayland, the first start should open the system screen-share picker. Press `ctrl+.` to request a clip. Current Phase 1 behavior depends on your FFmpeg build supporting the selected backend device.

Daemon smoke test:

```bash
printf '%s\n' '{"id":"1","cmd":"status"}' | cargo run -q -p skia-recorder
```
