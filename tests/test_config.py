import tempfile
import unittest
from pathlib import Path

from skia.config import load_config


class ConfigTests(unittest.TestCase):
    def test_loads_defaults_without_file(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            config = load_config(root=root, env={})

        self.assertEqual(config.clip_seconds, 30)
        self.assertEqual(config.segment_seconds, 2)
        self.assertEqual(config.backend, "auto")
        self.assertEqual(config.fps, 60)
        self.assertEqual(config.cache_dir, root / ".cache" / "skia")
        self.assertIsNone(config.video_input)
        self.assertIsNone(config.audio_input)
        self.assertEqual(config.hotkey, "<ctrl>+.")
        self.assertEqual(config.output_dir, root / "out")
        self.assertIsNone(config.gstreamer.bitrate_kbps)
        self.assertIsNone(config.gstreamer.quantizer)
        self.assertIsNone(config.gstreamer.x264_preset)
        self.assertIsNone(config.gstreamer.audio_bitrate_bps)

    def test_loads_project_config(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            root.joinpath("skia.toml").write_text(
                "\n".join(
                    [
                        "[recording]",
                        "clip_seconds = 45",
                        "segment_seconds = 3",
                        'backend = "linux-wayland-ffmpeg"',
                        "fps = 30",
                        'cache_dir = "cache"',
                        'video_input = "42"',
                        'audio_input = "default"',
                        "",
                        "[app]",
                        'hotkey = "<ctrl>+<shift>+."',
                        'output_dir = "clips"',
                    ]
                )
            )

            config = load_config(root=root, env={})

        self.assertEqual(config.clip_seconds, 45)
        self.assertEqual(config.segment_seconds, 3)
        self.assertEqual(config.backend, "linux-wayland-ffmpeg")
        self.assertEqual(config.fps, 30)
        self.assertEqual(config.cache_dir, root / "cache")
        self.assertEqual(config.video_input, "42")
        self.assertEqual(config.audio_input, "default")
        self.assertEqual(config.hotkey, "<ctrl>+<shift>+.")
        self.assertEqual(config.output_dir, root / "clips")

    def test_env_config_overrides_project_config(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            config_path = root / "custom.toml"
            config_path.write_text("[recording]\nclip_seconds = 15\n")

            config = load_config(root=root, env={"SKIA_CONFIG": str(config_path)})

        self.assertEqual(config.clip_seconds, 15)

    def test_loads_gstreamer_quality_overrides(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            root.joinpath("skia.toml").write_text(
                "\n".join(
                    [
                        "[gstreamer]",
                        "bitrate_kbps = 8000",
                        "quantizer = 18",
                        'x264_preset = "fast"',
                        "audio_bitrate_bps = 96000",
                    ]
                )
            )

            config = load_config(root=root, env={})

        self.assertEqual(config.gstreamer.bitrate_kbps, 8000)
        self.assertEqual(config.gstreamer.quantizer, 18)
        self.assertEqual(config.gstreamer.x264_preset, "fast")
        self.assertEqual(config.gstreamer.audio_bitrate_bps, 96000)

    def test_rejects_quantizer_out_of_range(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            root.joinpath("skia.toml").write_text("[gstreamer]\nquantizer = 60\n")

            with self.assertRaisesRegex(ValueError, "gstreamer.quantizer"):
                load_config(root=root, env={})

    def test_rejects_invalid_positive_ints(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            root.joinpath("skia.toml").write_text("[recording]\nclip_seconds = 0\n")

            with self.assertRaisesRegex(ValueError, "recording.clip_seconds"):
                load_config(root=root, env={})


if __name__ == "__main__":
    unittest.main()
