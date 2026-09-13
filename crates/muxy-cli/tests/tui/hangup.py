import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import sys
import termios
import time

pid, master = pty.fork()
if pid == 0:
    os.execv(sys.argv[1], [sys.argv[1]])
fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 26, 100, 0, 0))
exited = False
try:
    deadline = time.monotonic() + 10
    raw = b""
    while time.monotonic() < deadline:
        if select.select([master], [], [], 0.02)[0]:
            raw += os.read(master, 65536)
            if b"\x1b[6n" in raw:
                os.write(master, b"\x1b[1;1R")
                raw = raw.replace(b"\x1b[6n", b"")
        state_path = Path(os.environ["MUXY_DIR"]) / "tui-state.json"
        if state_path.exists():
            state = json.loads(state_path.read_text())
            tabs = state["projects"][state["active"]]["tabs"]
            if tabs and all(pane["session"] for pane in tabs[0]["panes"].values()):
                break
    else:
        raise AssertionError(f"TUI did not initialize: {raw[-1000:]!r}")
    # Let the input loop enter its idle poll before revoking the terminal.
    time.sleep(0.2)
    os.close(master)
    master = None
    deadline = time.monotonic() + 3
    while time.monotonic() < deadline:
        found, status = os.waitpid(pid, os.WNOHANG)
        if found:
            exited = True
            assert os.WIFEXITED(status), status
            break
        time.sleep(0.02)
    assert exited, "TUI remained alive after its host PTY closed (terminal EOF spin)"
finally:
    if not exited:
        os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)
    if master is not None:
        os.close(master)
