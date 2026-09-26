#!/usr/bin/env python3
"""A plugin sees new canvas dimensions when ttfx restarts it on SIGWINCH."""
import fcntl
import os
import pathlib
import pty
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

binary = sys.argv[1]
with tempfile.TemporaryDirectory() as tmp:
    plugin = pathlib.Path(tmp, 'ttfx-effect-resizeprobe')
    log = pathlib.Path(tmp, 'sizes')
    plugin.write_text('''#!/usr/bin/env python3
import os, struct, sys
sys.stdin.read()
w, h = int(os.environ["TTFX_CANVAS_WIDTH"]), int(os.environ["TTFX_CANVAS_HEIGHT"])
with open(os.environ["SIZE_LOG"], "a") as log:
    log.write(f"{w}x{h}\\n")
frame = (" " * w + "\\n") * (h - 1) + " " * w
data = frame.encode()
while True:
    sys.stdout.buffer.write(struct.pack("<I", len(data)) + data)
    sys.stdout.buffer.flush()
''')
    plugin.chmod(0o755)
    master, slave = pty.openpty()
    def size(width, height):
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', height, width, 0, 0))
    size(20, 8)
    env = os.environ.copy()
    env.pop('COLUMNS', None)
    env.pop('LINES', None)
    env.update(TTFX_EFFECT_PATH=tmp, SIZE_LOG=str(log))
    proc = subprocess.Popen([binary, '--frame-rate', '10', 'resizeprobe'], stdin=subprocess.DEVNULL,
                            stdout=slave, stderr=subprocess.PIPE, env=env)
    try:
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline and (not log.exists() or '20x8' not in log.read_text()):
            time.sleep(0.02)
        assert log.exists() and '20x8' in log.read_text(), 'plugin did not start at 20x8'
        size(30, 10)
        proc.send_signal(signal.SIGWINCH)
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline and '30x10' not in log.read_text():
            time.sleep(0.02)
        assert '30x10' in log.read_text(), log.read_text()
    finally:
        proc.send_signal(signal.SIGINT)
        proc.wait(timeout=3)
        os.close(master)
        os.close(slave)
