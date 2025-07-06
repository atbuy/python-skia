import os
import time
from datetime import datetime
from pathlib import Path
from threading import Thread

import blosc2
import imageio
import numpy as np
from mss import mss
from pynput import keyboard
from pynput.keyboard import Key, KeyCode


class Skia:
    def __init__(self):
        self.width = 1920
        self.height = 1080
        self.size = (self.width, self.height)

        self.framerate = 60
        self._mean_framerate = self.framerate
        self.video_length = 30

        self.buffer = np.array([None] * self.framerate * self.video_length)
        self.index = 0
        self.length = len(self.buffer)

    def _on_press(self, key: Key | KeyCode | None):
        if key is None or not isinstance(key, KeyCode):
            return

        # F13 is pressed
        if key.vk == 269025153:
            self._save()

    def _save(self):
        buffer = self.buffer.copy()

        th = Thread(target=self._store_buffer, args=(buffer,))
        th.start()

    def _store_buffer(self, buffer):
        """Store clip to specified path."""

        self._store_buffer_imageio(buffer)

        # Send notification after the file has been stored
        os.system("notify-send 'Skia' 'Stored clip'")

    def _store_buffer_imageio(self, buffer):
        """Store clip to specified path, using imageio."""

        filename = datetime.now().isoformat()
        outpath = Path(__file__).parent.parent.joinpath("out", f"{filename}.mp4")

        framerate = self._mean_total_frames // self._mean_counter
        writer = imageio.get_writer(outpath, fps=framerate)

        for compressed in buffer:
            if compressed is None:
                break

            decompressed = blosc2.unpack_array(compressed)
            writer.append_data(decompressed.astype("uint8"))

        writer.close()

    def start(self):
        """Start loop to store images."""

        print("Started loop...")

        sct = mss()
        monitor = sct.monitors[2]

        listener = keyboard.Listener(on_press=self._on_press)
        listener.start()

        try:
            prev = time.perf_counter()
            fps = 0

            self._mean_counter = 0
            self._mean_total_frames = 0

            while True:
                if (time.perf_counter() - prev) >= 1:
                    prev = time.perf_counter()
                    self._mean_counter += 1
                    self._mean_total_frames += fps
                    fps = 0
                else:
                    if fps >= self.framerate:
                        time.sleep(0.001)
                        continue

                screen = sct.grab(monitor)
                fps += 1

                # Convert screenshot to numpy array and convert from BGRA to RGB
                array = np.array(screen, dtype=np.uint8)
                array = np.flip(array[:, :, :3], 2)

                # Compress array and store it
                self.buffer[self.index] = blosc2.pack_array(array)

                self.index += 1

                if self.index == self.length:
                    self.index = self.length - 1
                    self.buffer = np.roll(self.buffer, -1)

        except KeyboardInterrupt:
            pass

        listener.stop()

        print("Stopped loop...")


def main():
    skia = Skia()
    skia.start()


if __name__ == "__main__":
    main()
