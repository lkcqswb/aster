"""Text snapshots and graceful teardown for PTY integration tests."""
import time
import hashlib
import pexpect
from wcwidth import wcwidth


class InlineFrames:
    """Strip complete image packets before pyte parses text; preserve split packets."""
    prefix = '\x1b]1337;File='

    def __init__(self, stream):
        self.stream = stream
        self.pending = ''
        self.frames = set()

    def feed(self, chunk):
        self.pending += chunk
        while self.pending:
            start = self.pending.find(self.prefix)
            if start < 0:
                keep = max((n for n in range(1, len(self.prefix))
                            if self.pending.endswith(self.prefix[:n])), default=0)
                if keep:
                    self.stream.feed(self.pending[:-keep])
                    self.pending = self.pending[-keep:]
                else:
                    self.stream.feed(self.pending)
                    self.pending = ''
                return
            self.stream.feed(self.pending[:start])
            self.pending = self.pending[start:]
            end = self.pending.find('\x07')
            if end < 0:
                assert len(self.pending) < 4_000_000, 'Unterminated image packet'
                return
            packet = self.pending[:end]
            payload = packet.split(':', 1)[1]
            self.frames.add(hashlib.sha256(payload.encode()).hexdigest())
            self.pending = self.pending[end + 1:]


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
