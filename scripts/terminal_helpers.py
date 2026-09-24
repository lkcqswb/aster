"""Text snapshots and graceful teardown for PTY integration tests."""
import time
import pexpect
from wcwidth import wcwidth


def screen_text(screen):
    rows = []
    for y in range(screen.lines):
        row = []
        continuation = False
        for x in range(screen.columns):
            if continuation:
                continuation = False
                continue
            # Resizing a dirty Ratatui frame can leave an isolated continuation
            # cell in pyte. Treat it as blank instead of indexing an empty string.
            data = screen.buffer[y][x].data or ' '
            row.append(data)
            continuation = wcwidth(data[0]) == 2
        rows.append(''.join(row))
    return '\n'.join(rows)


def stop_child(child, stream=None):
    if child is None or not child.isalive():
        return
    child.send('\x03')
    deadline = time.monotonic()+8
    while time.monotonic()<deadline:
        try:
            chunk = child.read_nonblocking(524288, timeout=.1)
            if stream is not None:
                stream.feed(chunk)
        except pexpect.TIMEOUT:
            pass
        except pexpect.EOF:
            child.close()
            return
    child.terminate(force=True)
