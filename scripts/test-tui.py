#!/usr/bin/env python3
"""Exercise the real explorer in a Unix pseudo-terminal using only stdlib."""

from __future__ import annotations

import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import random
import re
import select
import signal
import struct
import sys
import tempfile
import termios
import threading
import time
import unittest


BINARY = Path(sys.argv.pop(1) if len(sys.argv) > 1 else "target/debug/jgrep").resolve()


class ExplorerTerminal:
    def __init__(
        self, args: list[str], directory: Path, stdin: bytes | None = None,
        subcommand: str | None = "explore",
        width: int = 120, height: int = 30,
        environment: dict[str, str] | None = None,
        program: Path | None = None,
    ) -> None:
        env = dict(os.environ)
        env.pop('NO_COLOR', None)  # Color tests must not depend on the host's shell.
        env.update(
            TERM="xterm-256color",
            JGREP_FILTER_SAVE_PATH=str(directory / "saved.jq"),
            JGREP_FILTER_HISTORY_FILE=str(directory / "history"),
            JGREP_LAST_FILTER_FILE=str(directory / "last.jq"),
        )
        env.update(environment or {})
        stream_fd: int | None = None
        if stdin is not None:
            stream_fd, write_fd = os.pipe()
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            fcntl.ioctl(1, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
            if stream_fd is not None:
                os.close(write_fd)
                os.dup2(stream_fd, 0)
                os.close(stream_fd)
            executable = str(program or BINARY)
            command = [executable, *([subcommand] if subcommand else []), *args]
            os.execve(executable, command, env)
        if stream_fd is not None:
            os.close(stream_fd)
            def feed() -> None:
                try:
                    pending = memoryview(stdin)
                    while pending:
                        pending = pending[os.write(write_fd, pending):]
                except BrokenPipeError:
                    pass
                finally:
                    os.close(write_fd)
            # Feeding before fork deadlocks once stdin exceeds the pipe capacity.
            self.feeder = threading.Thread(target=feed, daemon=True)
            self.feeder.start()
        self.output = bytearray()
        self.status: int | None = None

    def resize(self, width: int, height: int) -> None:
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
        os.kill(self.pid, signal.SIGWINCH)

    def read(self, timeout: float) -> bool:
        if not select.select([self.fd], [], [], timeout)[0]:
            return True
        try:
            data = os.read(self.fd, 65536)
        except OSError as error:
            if error.errno != errno.EIO:
                raise
            return False
        self.output.extend(data)
        return bool(data)

    def screen_text(self) -> bytes:
        """Reconstruct fixture text from the CSI cursor/clear commands Ratatui emits."""
        rows: dict[int, dict[int, str]] = {}
        x = y = 0
        parts = re.split(r"(\x1b\[[0-?]*[ -/]*[@-~])", self.output.decode("utf-8", "replace"))
        for part in parts:
            if part.startswith("\x1b["):
                args, command = part[2:-1], part[-1]
                if args.startswith("?"):
                    continue  # Mode changes, including cursor visibility.
                numbers = [int(value or 0) for value in args.split(";")]
                if command in ("H", "f"):
                    y = max(1, numbers[0]) - 1
                    x = max(1, numbers[1] if len(numbers) > 1 else 1) - 1
                elif command == "J" and numbers[0] in (2, 3):
                    rows.clear()
                elif command == "K":
                    row = rows.setdefault(y, {})
                    for column in list(row):
                        if (
                            numbers[0] == 2
                            or (numbers[0] == 0 and column >= x)
                            or (numbers[0] == 1 and column <= x)
                        ):
                            del row[column]
                elif command in ("A", "B", "C", "D"):
                    distance = numbers[0] or 1
                    if command == "A":
                        y = max(0, y - distance)
                    elif command == "B":
                        y += distance
                    elif command == "C":
                        x += distance
                    else:
                        x = max(0, x - distance)
                continue
            for character in part:
                if character == "\r":
                    x = 0
                elif character == "\n":
                    y += 1
                elif character >= " ":
                    rows.setdefault(y, {})[x] = character
                    x += 1
        return "\n".join(
            "".join(row.get(column, " ") for column in range(max(row, default=-1) + 1))
            for _, row in sorted(rows.items())
        ).encode("utf-8")

    def wait_for(self, text: bytes, *, rendered: bool = False) -> None:
        def contains_text() -> bool:
            return text in (self.screen_text() if rendered else self.output)

        deadline = time.monotonic() + 10
        while not contains_text() and time.monotonic() < deadline:
            if not self.read(0.1):
                break
        if not contains_text():
            raise AssertionError(f"TUI did not show {text!r}: {bytes(self.output[-300:])!r}")

    def finish(self, keys: bytes, expected_code: int = 0) -> bytes:
        os.write(self.fd, keys)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if not self.read(0.1):
                _, self.status = os.waitpid(self.pid, 0)
                break
        if self.status is None:
            raise AssertionError("TUI did not exit within 10 seconds")
        if os.waitstatus_to_exitcode(self.status) != expected_code:
            raise AssertionError(f"TUI exited with {self.status}: {bytes(self.output[-300:])!r}")
        return bytes(self.output).rsplit(b"\x1b[?1049l", 1)[-1]

    def close(self) -> None:
        try:
            if self.status is None:
                os.kill(self.pid, signal.SIGKILL)
                deadline = time.monotonic() + 5
                while time.monotonic() < deadline:
                    pid, status = os.waitpid(self.pid, os.WNOHANG)
                    if pid:
                        self.status = status
                        break
                    time.sleep(0.01)
                if self.status is None:
                    raise AssertionError("Killed TUI child did not exit within 5 seconds")
        finally:
            os.close(self.fd)


class ExplorerSmokeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory(prefix="jgrep-tui-")
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name)
        self.input = self.path / "data.ndjson"
        self.input.write_text('{"name":"Alice"}\n{"name":"Bob"}\n', encoding="utf-8")

    def terminal(self, args: list[str], stdin: bytes | None = None) -> ExplorerTerminal:
        terminal = ExplorerTerminal(args, self.path, stdin)
        self.addCleanup(terminal.close)
        terminal.wait_for(b"Preview")
        return terminal

    def test_edit_save_and_export_filter(self) -> None:
        terminal = self.terminal([str(self.input), "-f", "name"])
        os.write(terminal.fd, b"\x13")  # Ctrl-S
        terminal.wait_for(b"saved")
        self.assertEqual((self.path / "saved.jq").read_text(), ".name\n")
        self.assertEqual((self.path / "history").read_text(), "name\n")
        self.assertIn(b".name\r\n", terminal.finish(b"\x19"))  # Ctrl-Y
        self.assertEqual((self.path / "last.jq").read_text(), ".name\n")

    def test_edit_output_toggle_and_print(self) -> None:
        terminal = self.terminal([str(self.input)])
        os.write(terminal.fd, b"\x0fname\x07")  # Ctrl-O, edit Output, Ctrl-G layout
        terminal.wait_for(b"full")
        result = terminal.finish(b"\x06\x0c\x0b\r")  # Pretty, level, no color, print
        self.assertIn(b"Alice\r\nBob\r\n", result)

    def test_hostile_controls_are_encoded_in_colored_preview(self) -> None:
        payload = "before\x1b]52;c;synthetic\x07\r\t\x9bafter"
        self.input.write_text(json.dumps({"level": "ERROR", "message": payload}) + "\n", encoding="utf-8")
        terminal = self.terminal([str(self.input), "-C", "-p", "message"])
        terminal.wait_for(b"\\u{1b}]52", rendered=True)
        self.assertNotIn(b"\x1b]52;", terminal.output)
        self.assertNotIn(b"\x07", terminal.output)
        self.assertNotIn("\x9b".encode(), terminal.output)
        terminal.finish(b"\x1b")

    def test_hostile_field_completion_is_literal_and_exportable(self) -> None:
        key = "select(true) | 42"
        self.input.write_text(json.dumps({key: "literal-result"}) + "\n", encoding="utf-8")
        terminal = self.terminal([str(self.input)])
        os.write(terminal.fd, b"s\t\x13")
        terminal.wait_for(b"saved")
        saved = (self.path / "saved.jq").read_text()
        self.assertEqual(saved, '.["select(true) | 42"]\n')
        self.assertIn(b"literal-result\r\n", terminal.finish(b"\r"))

    def test_buffered_stream_keeps_every_record(self) -> None:
        terminal = self.terminal(
            ["-p", "name"],
            stdin=b'{"name":"first"}\n{"name":"second"}\n{"name":"third"}\n',
        )
        terminal.wait_for(b"complete")
        self.assertIn(b"first\r\nsecond\r\nthird\r\n", terminal.finish(b"\r"))

    def test_seeded_filter_output_matrix_in_real_terminal(self) -> None:
        randomizer = random.Random(0x4A67726570)
        for case in range(24):
            threshold = randomizer.randrange(100)
            records = [{"score": randomizer.randrange(100), "message": f"row-{index}"}
                       for index in range(randomizer.randrange(5, 40))]
            payload = ''.join(json.dumps(record) + '\n' for record in records).encode()
            filter_text = randomizer.choice([f"score>={threshold}", f".score>={threshold}",
                                            f"select(.score >= {threshold})"])
            output_text = randomizer.choice(["message", ".message"])
            with self.subTest(case=case, filter=filter_text, output=output_text):
                if case % 2:
                    self.input.write_bytes(payload)
                    terminal = self.terminal([str(self.input), '-f', filter_text])
                else:
                    terminal = self.terminal(['-f', filter_text], payload)
                    terminal.wait_for(b'complete', rendered=True)
                os.write(terminal.fd, b'\x0f' + output_text.encode() + b'\x13')
                terminal.wait_for(b'saved', rendered=True)
                expected = ''.join(record['message'] + '\r\n' for record in records
                                   if record['score'] >= threshold).encode()
                result = terminal.finish(b'\r', expected_code=0 if expected else 1)
                self.assertEqual(result.removeprefix(b'\x1b[?25h'), expected)

    def test_completion_condition_and_invalid_filter_recovery(self) -> None:
        self.input.write_text('{"level":"ERROR","message":"match"}\n{"level":"INFO","message":"skip"}\n')
        terminal = self.terminal([str(self.input)])
        os.write(terminal.fd, b'le\t=ERROR\x0fmessage\x13')
        terminal.wait_for(b'saved', rendered=True)
        self.assertIn('select(.level == "ERROR")', (self.path / 'saved.jq').read_text())
        self.assertEqual(terminal.finish(b'\r').removeprefix(b'\x1b[?25h'), b'match\r\n')
        terminal = self.terminal([str(self.input), '-f', '.[' ])
        os.write(terminal.fd, b'\r\x13\x19')  # Invalid expressions must not exit/save/export.
        terminal.wait_for(b'no valid jq to export', rendered=True)
        os.write(terminal.fd, b'\x7f')
        terminal.wait_for(b'2 matches', rendered=True)
        terminal.finish(b'\r')

    def test_large_stream_tail_and_complete_export(self) -> None:
        count = 50000
        payload = ''.join(json.dumps({'index': i, 'message': f'row-{i}'}) + '\n'
                          for i in range(count)).encode()
        terminal = self.terminal(['-f', 'index>=49960', '-p', 'message'], payload)
        terminal.wait_for(b'stream complete: 50000 docs', rendered=True)
        terminal.wait_for(b'row-49999', rendered=True)
        terminal.wait_for(b'40 matches', rendered=True)
        expected = ''.join(f'row-{i}\r\n' for i in range(49960, count)).encode()
        self.assertEqual(terminal.finish(b'\r').removeprefix(b'\x1b[?25h'), expected)

    def test_mixed_types_preserve_valid_results_and_report_errors(self) -> None:
        terminal = self.terminal(['-p', '.message | ascii_downcase'],
                                 b'{"message":"HELLO"}\n{"message":[]}\n{"message":"WORLD"}\n')
        terminal.wait_for(b'complete', rendered=True)
        terminal.wait_for(b'1 eval errors', rendered=True)
        result = terminal.finish(b'\r', expected_code=2)
        self.assertIn(b'hello\r\nworld\r\n', result)
        self.assertIn(b'skipped incompatible records', result)

    def test_wrapped_colored_live_tail_remains_visible_after_resize(self) -> None:
        records = [{'level': 'ERROR', 'message': 'x' * 200 + f' TAIL_{i}'} for i in range(3)]
        payload = ''.join(json.dumps(record) + '\n' for record in records).encode()
        terminal = ExplorerTerminal(['-C', '-p', 'message'], self.path, payload, width=40, height=20)
        self.addCleanup(terminal.close)
        terminal.wait_for(b'stream complete: 3 docs', rendered=True)
        terminal.wait_for(b'TAIL_2', rendered=True)
        self.assertIn(b'38;2;255;80;90', terminal.output)  # Ratatui owns colored rows.
        for width, height in [(100, 24), (35, 18), (70, 25), (80, 18), (40, 20)]:
            terminal.resize(width, height)
            terminal.wait_for(b'TAIL_2', rendered=True)
        os.write(terminal.fd, b'\x1b[5~')  # PageUp pauses and moves through physical rows.
        terminal.wait_for(b'paused:3', rendered=True)
        os.write(terminal.fd, b'\x14')
        terminal.wait_for(b'tail:3', rendered=True)
        terminal.wait_for(b'TAIL_2', rendered=True)
        os.write(terminal.fd, b'\x0b')  # Disable colors before raw export.
        terminal.wait_for(b'color off', rendered=True)
        expected = ''.join(record['message'] + '\r\n' for record in records).encode()
        self.assertEqual(terminal.finish(b'\r').removeprefix(b'\x1b[?25h'), expected)

    def test_seeded_resize_and_keyboard_navigation_preserve_export(self) -> None:
        randomizer = random.Random(0x5253495A45)
        records = [{'message': 'log-' + str(i) + '-' + 'x' * randomizer.randrange(1, 250)}
                   for i in range(20)]
        payload = ''.join(json.dumps(record) + '\n' for record in records).encode()
        terminal = ExplorerTerminal(['-p', 'message'], self.path, payload, width=80, height=24)
        self.addCleanup(terminal.close)
        terminal.wait_for(b'stream complete: 20 docs', rendered=True)
        for _ in range(40):
            terminal.resize(randomizer.randrange(36, 141), randomizer.randrange(16, 41))
            keys = randomizer.choice([b'\x1b[5~', b'\x1b[6~', b'\x07', b'\x14', b'\x0f'])
            os.write(terminal.fd, keys)
            terminal.read(.02)
        expected = ''.join(record['message'] + '\r\n' for record in records).encode()
        self.assertEqual(terminal.finish(b'\r').removeprefix(b'\x1b[?25h'), expected)

    def test_grapheme_backspace_does_not_leave_half_an_emoji_in_editor(self) -> None:
        terminal = self.terminal([str(self.input)])
        os.write(terminal.fd, '👩‍💻'.encode() + b'\x7fname\x19')
        self.assertIn(b'.name\r\n', terminal.finish(b''))

    def test_no_color_startup_and_explicit_toggle_control_the_renderer(self) -> None:
        terminal = ExplorerTerminal(['-C', '-p', 'message'], self.path,
                                    b'{"level":"ERROR","message":"test-colors"}\n',
                                    environment={'NO_COLOR': '1'})
        self.addCleanup(terminal.close)
        terminal.wait_for(b'test-colors', rendered=True)
        self.assertNotIn(b'38;2;255;80;90', terminal.output)
        os.write(terminal.fd, b'\x0b')
        terminal.wait_for(b'38;2;255;80;90')
        terminal.finish(b'\x1b')

    def test_escape_restores_terminal(self) -> None:
        terminal = self.terminal([str(self.input)])
        terminal.finish(b"\x1b")
        self.assertIn(b"\x1b[?1049l", terminal.output)

    def test_narrow_help_preserves_filter_and_resize_restores_fields(self) -> None:
        terminal = ExplorerTerminal(
            [str(self.input), "-f", "name"], self.path, width=40, height=20,
        )
        self.addCleanup(terminal.close)
        terminal.wait_for(b"F1 help | Ctrl-O input | Enter | Esc", rendered=True)
        self.assertNotIn(b"Fields", terminal.output)
        os.write(terminal.fd, b"\x1bOP")  # F1
        terminal.wait_for(b"Explorer help", rendered=True)
        terminal.wait_for(b"F1/Esc close help", rendered=True)
        os.write(terminal.fd, b"?\r\x13\x1b[6~")  # Ignore edit, print/save, scroll help
        terminal.wait_for(b"Ctrl-S: save generated jq", rendered=True)
        self.assertFalse((self.path / "saved.jq").exists())
        terminal.resize(100, 24)
        os.write(terminal.fd, b"\x1b")  # Esc closes help, not the explorer
        terminal.wait_for(b"Fields", rendered=True)
        self.assertIn(b"Alice\r\nBob\r\n", terminal.finish(b"\r"))

    def test_narrow_stream_keeps_status_and_help_visible(self) -> None:
        terminal = ExplorerTerminal(
            ["-p", "name"], self.path,
            stdin=b'{"name":"first"}\n{"name":"second"}\n', width=40, height=20,
        )
        self.addCleanup(terminal.close)
        terminal.wait_for(b"complete")
        terminal.wait_for(b"F1 help | Ctrl-O input | Enter | Esc", rendered=True)
        self.assertIn(b"first\r\nsecond\r\n", terminal.finish(b"\r"))

    def test_colored_preview_does_not_paint_over_help(self) -> None:
        self.input.write_text('{"level":"WARN","message":"check overlay"}\n', encoding="utf-8")
        terminal = self.terminal([str(self.input), "-C"])
        terminal.wait_for(b"WARN", rendered=True)
        os.write(terminal.fd, b"\x1bOP")
        terminal.wait_for(b"F1/Esc close help", rendered=True)
        self.assertNotIn(b"WARN", terminal.screen_text())
        terminal.finish(b"\x1bOP\x1b")

    def test_k9s_bridge_supports_real_tui_save_and_idle_cleanup(self) -> None:
        kubectl = self.path / "kubectl"
        pid_file = self.path / "kubectl.pid"
        kubectl.write_text(
            f"#!{sys.executable}\nimport os, time\nfrom pathlib import Path\n"
            f"Path({str(pid_file)!r}).write_text(str(os.getpid()))\n"
            "print('{\"level\":\"ERROR\",\"message\":\"declined\"}', flush=True)\n"
            "print('plain log line', flush=True)\ntime.sleep(60)\n",
            encoding="utf-8",
        )
        kubectl.chmod(0o700)
        terminal = ExplorerTerminal(
            [
                "logs",
                "--context=test-context", "--namespace=shop", "--name=test-pod",
                f"--kubectl={kubectl}", f"--jgrep={BINARY}",
                f"--state-dir={self.path / 'state'}",
            ],
            self.path, subcommand="k9s",
        )
        self.addCleanup(terminal.close)
        terminal.wait_for(b"plain log line", rendered=True)
        os.write(terminal.fd, b"\x1bOP")
        terminal.wait_for(b"F1/Esc close help", rendered=True)
        os.write(terminal.fd, b"\x1bOP\x0fmessage\x13")
        terminal.wait_for(b"saved", rendered=True)
        self.assertEqual((self.path / "state/saved-filter.jq").read_text(), ".message\n")
        os.write(terminal.fd, b'\x0b')
        terminal.wait_for(b'color off', rendered=True)
        self.assertIn(b"declined\r\nplain log line\r\n", terminal.finish(b"\r"))
        with self.assertRaises(ProcessLookupError):
            os.kill(int(pid_file.read_text()), 0)

    def test_k9s_live_tail_pause_resume_and_full_export(self) -> None:
        kubectl = self.path / "kubectl-tail"
        pid_file = self.path / "tail.pid"
        release = self.path / "next-log"
        kubectl.write_text(
            f"#!{sys.executable}\nimport json, os, time\nfrom pathlib import Path\n"
            f"Path({str(pid_file)!r}).write_text(str(os.getpid()))\n"
            "for i in range(120):\n"
            " print(json.dumps({'level':'ERROR' if i%3==0 else 'INFO', 'message':f'm{i:06d}'}), flush=True)\n"
            "print('plain log line', flush=True)\n"
            f"while not Path({str(release)!r}).exists(): time.sleep(.02)\n"
            "print('{\"level\":\"ERROR\",\"message\":\"LATEST-MARKER\"}', flush=True)\n"
            "time.sleep(60)\n", encoding="utf-8",
        )
        kubectl.chmod(0o700)
        terminal = ExplorerTerminal(
            ["logs", "--context=test", "--namespace=synthetic", "--name=test-pod",
             f"--kubectl={kubectl}", f"--jgrep={BINARY}", f"--state-dir={self.path / 'tail-state'}"],
            self.path, subcommand="k9s",
        )
        self.addCleanup(terminal.close)
        terminal.wait_for(b'stream live: 121 docs', rendered=True)
        os.write(terminal.fd, b'level==ERROR\x0fmessage')
        terminal.wait_for(b'40 matches', rendered=True)
        terminal.wait_for(b'm000117', rendered=True)
        os.write(terminal.fd, b'\x14')  # Ctrl-T freezes the preview, not ingestion.
        terminal.wait_for(b'paused:30', rendered=True)
        release.touch()
        terminal.wait_for(b'stream live: 122 docs', rendered=True)
        self.assertNotIn(b'LATEST-MARKER', terminal.screen_text())
        os.write(terminal.fd, b'\x14')
        terminal.wait_for(b'LATEST-MARKER', rendered=True)
        os.write(terminal.fd, b'\x0b')
        terminal.wait_for(b'color off', rendered=True)
        result = terminal.finish(b'\r').removeprefix(b'\x1b[?25h')
        expected = ''.join(f'm{i:06d}\r\n' for i in range(0, 120, 3)).encode() + b'LATEST-MARKER\r\n'
        self.assertEqual(result, expected)
        with self.assertRaises(ProcessLookupError):
            os.kill(int(pid_file.read_text()), 0)

    def test_k9s_silent_start_allows_filter_output_help_and_later_logs(self) -> None:
        kubectl = self.path / "kubectl-silent"
        pid_file = self.path / "silent.pid"
        first, second = self.path / "first-batch", self.path / "second-batch"
        kubectl.write_text(
            f"#!{sys.executable}\nimport os, time\nfrom pathlib import Path\n"
            f"Path({str(pid_file)!r}).write_text(str(os.getpid()))\n"
            f"while not Path({str(first)!r}).exists(): time.sleep(.02)\n"
            "print('{\"level\":\"INFO\",\"message\":\"ignored\"}', flush=True)\n"
            "print('{\"level\":\"ERROR\",\"message\":\"FIRST-LIVE\"}', flush=True)\n"
            f"while not Path({str(second)!r}).exists(): time.sleep(.02)\n"
            "print('{\"level\":\"ERROR\",\"message\":\"SECOND-LIVE\"}', flush=True)\n"
            "time.sleep(60)\n", encoding="utf-8",
        )
        kubectl.chmod(0o700)
        terminal = ExplorerTerminal(
            ["logs", "--context=test", "--namespace=synthetic", "--name=silent-pod",
             f"--kubectl={kubectl}", f"--state-dir={self.path / 'silent-state'}"],
            self.path, subcommand="k9s",
        )
        self.addCleanup(terminal.close)
        terminal.wait_for(b"waiting for matching logs", rendered=True)
        os.write(terminal.fd, b"level=ERROR\x0fmessage")
        terminal.wait_for(b"level=ERROR", rendered=True)
        first.touch()
        terminal.wait_for(b"FIRST-LIVE", rendered=True)
        terminal.wait_for(b"stream live: 2 docs", rendered=True)
        self.assertNotIn(b"ignored", terminal.screen_text())
        os.write(terminal.fd, b"\x1bOP")
        terminal.wait_for(b"Explorer help", rendered=True)
        second.touch()
        os.write(terminal.fd, b"\x1bOP\x07")  # Close help and maximize preview.
        terminal.wait_for(b"SECOND-LIVE", rendered=True)
        terminal.wait_for(b"stream live: 3 docs", rendered=True)
        terminal.resize(40, 20)
        terminal.wait_for(b"SECOND-LIVE", rendered=True)
        os.write(terminal.fd, b"\x13\x0b")
        terminal.wait_for(b"color off", rendered=True)
        self.assertEqual(
            (self.path / "silent-state/saved-filter.jq").read_text(),
            'select(.level == "ERROR") | .message\n',
        )
        result = terminal.finish(b"\r").removeprefix(b"\x1b[?25h")
        self.assertEqual(result, b"FIRST-LIVE\r\nSECOND-LIVE\r\n")
        with self.assertRaises(ProcessLookupError):
            os.kill(int(pid_file.read_text()), 0)

    def test_k9s_silent_start_can_be_closed_without_any_log(self) -> None:
        kubectl = self.path / "kubectl-empty"
        pid_file = self.path / "empty.pid"
        kubectl.write_text(
            f"#!{sys.executable}\nimport os, time\nfrom pathlib import Path\n"
            f"Path({str(pid_file)!r}).write_text(str(os.getpid()))\ntime.sleep(60)\n",
            encoding="utf-8",
        )
        kubectl.chmod(0o700)
        terminal = ExplorerTerminal(
            ["logs", "--context=test", "--namespace=synthetic", "--name=empty-pod",
             f"--kubectl={kubectl}", f"--state-dir={self.path / 'empty-state'}"],
            self.path, subcommand="k9s",
        )
        self.addCleanup(terminal.close)
        terminal.wait_for(b"waiting for matching logs", rendered=True)
        deadline = time.monotonic() + 5
        while not pid_file.exists() and time.monotonic() < deadline:
            terminal.read(.02)
        self.assertTrue(pid_file.exists())
        terminal.finish(b"\x1b")
        with self.assertRaises(ProcessLookupError):
            os.kill(int(pid_file.read_text()), 0)

    def test_stream_input_limit_returns_error(self) -> None:
        terminal = self.terminal(
            ["-p", "name", "--max-input-bytes", "30"],
            stdin=b'{"name":"first"}\n{"name":"second"}\n',
        )
        terminal.wait_for(b"complete")
        result = terminal.finish(b"\r", expected_code=2)
        self.assertIn(b"first\r\n", result)
        self.assertIn(b"larger than --max-input-bytes", result)

    def test_malformed_stream_returns_error(self) -> None:
        terminal = self.terminal(
            ["-p", "name"], stdin=b'{"name":"first"}\n{"name":\n'
        )
        terminal.wait_for(b"complete")
        result = terminal.finish(b"\r", expected_code=2)
        self.assertIn(b"first\r\n", result)
        self.assertIn(b"parse error", result)

    def test_cli_without_arguments_shows_help(self) -> None:
        terminal = ExplorerTerminal([], self.path, subcommand=None)
        self.addCleanup(terminal.close)
        terminal.wait_for(b"Examples:")
        terminal.finish(b"")

    def test_cli_without_input_explains_how_to_continue(self) -> None:
        terminal = ExplorerTerminal(["-p", "name"], self.path, subcommand=None)
        self.addCleanup(terminal.close)
        terminal.wait_for(b"no input provided")
        result = terminal.finish(b"", expected_code=2)
        self.assertIn(b"--null-input", result)

    def test_terminal_restores_raw_mode_for_buffered_and_streaming_input(self) -> None:
        for args, stdin in [([str(self.input)], None), ([], b'{"name":"stream"}\n')]:
            with self.subTest(streaming=stdin is not None):
                terminal = self.terminal(args, stdin)
                terminal.finish(b"\x1b")
                flags = termios.tcgetattr(terminal.fd)[3]
                self.assertTrue(flags & termios.ICANON)
                self.assertTrue(flags & termios.ECHO)


if __name__ == "__main__":
    unittest.main()
