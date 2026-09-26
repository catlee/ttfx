#!/usr/bin/env python3
"""Check Factorio plugin input rendering and framed output."""
import os
import re
import struct
import subprocess

PLUGIN = "plugins/ttfx-effect-factorio"
ANSI = re.compile(r"\x1b\[[0-9;]*m")


def frames(source, width=80, height=24, fps=12, cycles=1):
    env = dict(os.environ, TTFX_CANVAS_WIDTH=str(width),
               TTFX_CANVAS_HEIGHT=str(height), TTFX_FRAME_RATE=str(fps))
    output = subprocess.check_output([PLUGIN, "--cycles", str(cycles)],
                                     input=source.encode(), env=env)
    result = []
    while output:
        size = struct.unpack("<I", output[:4])[0]
        frame = output[4:4 + size].decode()
        assert len(frame.encode()) == size
        rows = ANSI.sub("", frame).split("\n")
        assert len(rows) == height
        assert all(len(row) == width for row in rows)
        result.append((frame, rows))
        output = output[4 + size:]
    return result


fallback = frames("")
assert len(fallback) > 20
assert "OMARCHY" not in "\n".join(fallback[-1][1])
assert "\x1b[38;2;" in fallback[-1][0]
assert "[]" in "\n".join(fallback[0][1])
assert "\x1b[38;2;163;167;82m" in fallback[-1][0]
assert fallback[-1][0] != fallback[-2][0]

plain = frames("FACTORIO")
assert "FACTORIO" in "\n".join(plain[-1][1])
assert plain[-1][0] != plain[-2][0]

art = frames("██  \n ▀█\n")
assert any("██" in row for row in art[-1][1])
assert any(" ▀█" in row for row in art[-1][1])
colored = frames("\033[38;2;255;0;0m█\033[0m \a▀")
assert any("█ ▀" in row for row in colored[-1][1])
assert "\033[38;2;255;0;0m" not in colored[-1][0]
assert any("A   B" in row for row in frames("A\a\tB")[-1][1])

for width, height in ((40, 12), (20, 8), (8, 4)):
    frames("FACTORIO", width, height)
assert "OMARCHY" in "\n".join(frames("", 20, 8)[-1][1])
assert "OMARCHY" in "\n".join(frames("", 8, 4)[-1][1])

one = frames("Z", fps=2)
two = frames("Z", fps=2, cycles=2)
assert len(two) == 2 * len(one)
