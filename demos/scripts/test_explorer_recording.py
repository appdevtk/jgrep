"""Focused checks for the demo-only key and terminal-size adapter."""

import os
import pty
import unittest

from explorer_recording import set_size, translate_keys


class RecordingAdapterTests(unittest.TestCase):
    def test_ordinary_editor_keys_pass_through(self) -> None:
        data = b"name?\r\x1b\x13\x19\x1b[6~"
        self.assertEqual(translate_keys(data), data)

    def test_recording_chord_injects_only_real_f1_sequence(self) -> None:
        self.assertEqual(translate_keys(b"\x12name\x12"), b"\x1bOPname\x1bOP")

    def test_terminal_size_switches_between_narrow_and_wide(self) -> None:
        master, slave = pty.openpty()
        try:
            for columns in [40, 100, 40]:
                set_size(master, columns, 20)
                self.assertEqual(os.get_terminal_size(slave), (columns, 20))
        finally:
            os.close(master)
            os.close(slave)


if __name__ == "__main__":
    unittest.main()
