#!/usr/bin/env bash
set -eu
cd "$(dirname "$0")/../.."
RUST=${RUST:-./target/release/ttfx}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cat > "$tmp/ttfx-effect-probe" <<'PY'
#!/usr/bin/env python3
import os, struct, sys
text = sys.stdin.read()
assert text == 'MARCH', repr(text)
assert os.environ['TTFX_EFFECT_PROTOCOL'] == '1'
assert (os.environ['TTFX_CANVAS_WIDTH'], os.environ['TTFX_CANVAS_HEIGHT']) == ('20', '8')
assert os.environ['TTFX_FRAME_RATE'] == '0'
assert sys.argv[1:] == ['--check', 'value']
frame = ('M' * 20 + '\n') * 7 + 'H' * 20
payload = frame.encode()
sys.stdout.buffer.write(struct.pack('<I', len(payload)) + payload)
PY
chmod +x "$tmp/ttfx-effect-probe"
printf MARCH | TTFX_EFFECT_PATH="$tmp" COLUMNS=20 LINES=8 "$RUST" --frame-rate 0 --parity-dump probe --check value > "$tmp/out" 2> "$tmp/err"
python3 - "$tmp/out" <<'PY'
import pathlib, sys
out = pathlib.Path(sys.argv[1]).read_bytes()
length, body = out.split(b'\n', 1)
assert len(body) == int(length) + 1
assert body.endswith(b'H' * 20 + b'\n')
PY
grep -q 'frames=1' "$tmp/err"
if printf MARCH | TTFX_EFFECT_PATH="$tmp" COLUMNS=20 LINES=8 "$RUST" --frame-rate 0 probe --check value > "$tmp/tty" 2> "$tmp/err"; then
  grep -q 'M\{20\}' "$tmp/tty"
  grep -q 'H\{20\}' "$tmp/tty"
  grep -Fq $'\033[?25h' "$tmp/tty"
else
  cat "$tmp/err" >&2; exit 1
fi
cat > "$tmp/ttfx-effect-fail" <<'PY'
#!/usr/bin/env python3
import sys
sys.stdin.read()
sys.exit(7)
PY
chmod +x "$tmp/ttfx-effect-fail"
if printf MARCH | TTFX_EFFECT_PATH="$tmp" "$RUST" fail > "$tmp/out" 2> "$tmp/err"; then
  echo 'failed plugin was accepted' >&2; exit 1
fi
grep -q "plugin 'fail' failed" "$tmp/err"
cat > "$tmp/ttfx-effect-badframe" <<'PY'
#!/usr/bin/env python3
import struct, sys
sys.stdin.read()
sys.stdout.buffer.write(struct.pack('<I', 20) + b'partial')
PY
chmod +x "$tmp/ttfx-effect-badframe"
if printf MARCH | TTFX_EFFECT_PATH="$tmp" "$RUST" badframe > "$tmp/out" 2> "$tmp/err"; then
  echo 'truncated plugin frame was accepted' >&2; exit 1
fi
grep -q 'plugin protocol:' "$tmp/err"
TTFX_EFFECT_PATH="$PWD/plugins" COLUMNS=90 LINES=24 "$RUST" --parity-dump --max-frames 2 factorio --cycles 1 </dev/null > "$tmp/out" 2> "$tmp/err"
grep -q 'frames=2' "$tmp/err"
python3 - "$tmp/out" <<'PY'
import pathlib, sys
out = pathlib.Path(sys.argv[1]).read_bytes()
for _ in range(2):
    line, out = out.split(b'\n', 1)
    frame, out = out[:int(line)], out[int(line) + 1:]
    assert len(frame.split(b'\n')) == 24
    assert b'\033[38;2;' in frame
    assert b'*' in frame
assert not out
PY
python3 tools/tests/plugin_resize.py "$RUST"
python3 tools/tests/factorio_plugin.py
printf 'FACTORIO' > "$tmp/input"
TTFX_EFFECT_PATH="$PWD/plugins" COLUMNS=80 LINES=24 "$RUST" -i "$tmp/input" --frame-rate 12 --parity-dump --max-frames 3 factorio --cycles 1 > "$tmp/out" 2> "$tmp/err"
grep -q 'frames=3' "$tmp/err"
python3 - "$tmp/out" <<'PY'
import pathlib, re, sys
out = pathlib.Path(sys.argv[1]).read_bytes()
for _ in range(3):
    line, out = out.split(b'\n', 1)
    frame, out = out[:int(line)], out[int(line) + 1:]
assert b'F' in re.sub(rb'\033\[[0-9;]*m', b'', frame)
assert not out
PY
