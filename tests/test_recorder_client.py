import sys
import tempfile
import unittest
from pathlib import Path

from skia.recorder_client import RecorderClient


class RecorderClientTests(unittest.TestCase):
    def test_reports_unexpected_daemon_exit(self):
        events = []
        with tempfile.TemporaryDirectory() as directory:
            client = RecorderClient(
                [
                    sys.executable,
                    "-c",
                    "import sys; print('{\"event\":\"ready\",\"version\":\"test\"}'); sys.stdout.flush(); sys.exit(7)",
                ],
                cwd=Path(directory),
                on_event=events.append,
            )

            client.start_process()
            ready = client.wait_for_event(timeout=2)
            exited = client.wait_for_event(timeout=2)
            client.close()

        self.assertEqual(ready, {"event": "ready", "version": "test"})
        self.assertEqual(exited["event"], "error")
        self.assertEqual(exited["code"], "daemon_exited")
        self.assertIn("7", exited["message"])
        self.assertIn(exited, events)

    def test_close_does_not_report_daemon_exit(self):
        events = []
        with tempfile.TemporaryDirectory() as directory:
            client = RecorderClient(
                [
                    sys.executable,
                    "-c",
                    "import time; time.sleep(10)",
                ],
                cwd=Path(directory),
                on_event=events.append,
            )

            client.start_process()
            client.close()

        self.assertEqual(events, [])


if __name__ == "__main__":
    unittest.main()
