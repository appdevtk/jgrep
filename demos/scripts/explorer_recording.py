#!/usr/bin/env python3
"""Demo-only PTY adapter for VHS 0.11, which cannot press function keys.

Ctrl-R injects the real F1 sequence. Ctrl-T toggles a 40/100-column PTY.
All other keys pass unchanged to the production jgrep binary.
"""

from __future__ import annotations

import argparse
import errno
import fcntl
import os
from pathlib import Path
import pty
import select
import signal
import struct
import tempfile
import termios
import tty


def translate_keys(data: bytes) -> bytes:
    return data.replace(b"\x12", b"\x1bOP")


def set_size(fd: int, columns: int, rows: int) -> None:
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--narrow", action="store_true")
    parser.add_argument("args", nargs=argparse.REMAINDER)
    options = parser.parse_args()
    arguments = options.args[1:] if options.args[:1] == ["--"] else options.args
    original = termios.tcgetattr(0)
    outer = os.get_terminal_size(0)
    wide_columns = min(100, outer.columns)
    columns = 40 if options.narrow else wide_columns
    rows = min(20 if options.narrow else 30, outer.lines)
    binary = Path("target/debug/jgrep").resolve()
    with tempfile.TemporaryDirectory(prefix="jgrep-recording-") as scratch:
        env = dict(os.environ)
        env.update(
            JGREP_FILTER_HISTORY_FILE=str(Path(scratch) / "history"),
            JGREP_LAST_FILTER_FILE=str(Path(scratch) / "last.jq"),
            JGREP_FILTER_SAVE_PATH=str(Path("demos/out/explorer-filter.jq").resolve()),
        )
        pid, fd = pty.fork()
        if pid == 0:
            set_size(1, columns, rows)
            os.execve(str(binary), [str(binary), "explore", *arguments], env)
        reaped = False
        try:
            tty.setraw(0)
            while True:
                ready, _, _ = select.select([0, fd], [], [])
                if fd in ready:
                    try:
                        data = os.read(fd, 65536)
                    except OSError as error:
                        if error.errno != errno.EIO:
                            raise
                        break
                    if not data:
                        break
                    os.write(1, data)
                if 0 in ready:
                    data = os.read(0, 4096)
                    if not data:
                        break
                    # Preserve ordering when a resize and ordinary keys share a read.
                    for index, chunk in enumerate(data.split(b"\x14")):
                        if index:
                            columns = wide_columns if columns == 40 else 40
                            set_size(fd, columns, rows)
                            os.kill(pid, signal.SIGWINCH)
                        if chunk:
                            os.write(fd, translate_keys(chunk))
            _, status = os.waitpid(pid, 0)
            reaped = True
            return os.waitstatus_to_exitcode(status)
        finally:
            termios.tcsetattr(0, termios.TCSADRAIN, original)
            os.close(fd)
            if not reaped:
                os.kill(pid, signal.SIGKILL)
                os.waitpid(pid, 0)


if __name__ == "__main__":
    raise SystemExit(main())
