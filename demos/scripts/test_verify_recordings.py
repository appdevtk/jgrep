"""Exercise the media checker against real ffprobe, including missing input."""

from pathlib import Path
import subprocess
import tempfile
import unittest

from verify_recordings import probe


class MediaVerificationTests(unittest.TestCase):
    def test_generated_video_contains_decodable_frames(self) -> None:
        info = probe(Path("demos/out/quickstart.mp4"))
        self.assertEqual((info.width, info.height), (1200, 720))
        self.assertGreater(info.frames, 1)
        self.assertGreater(info.duration, 0)

    def test_missing_video_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="jgrep-media-check-") as scratch:
            with self.assertRaises(subprocess.CalledProcessError):
                probe(Path(scratch) / "missing.mp4")


if __name__ == "__main__":
    unittest.main()
