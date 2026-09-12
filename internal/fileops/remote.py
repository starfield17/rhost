"""One-shot remote file operations, used by `rhost fs read/write/patch`.

The program is embedded into the rhost binary and executed by a *remote* python3
(AGENTS.md §5: nothing is installed here, a missing interpreter is reported as a
dependency failure instead). It reads one JSON request on stdin and writes one
JSON response on stdout, so no path, pattern or file content ever has to survive
a shell parser, and the CLI needs only one transport (docs/architecture/files-and-json.md).

Stability rules this file has to keep:

* `error` is always one of the codes in internal/errs; an unexpected exception
  becomes `INTERNAL`, never a guess;
* a response is bounded by the request's own `max_bytes`, so a large file or a
  large read can never make the CLI drain an unbounded stream;
* nothing is written unless the request's hash precondition still matches.
"""
import base64
import contextlib
import fcntl
import hashlib
import json
import os
import stat
import sys
import tempfile

# Editing limit, in bytes, for a whole file. Reads are bounded by
# the caller's max_bytes instead; this bounds *writes*, where the whole new body
# has to be held in memory to be hashed and compared.
EDIT_LIMIT = 8 * 1024 * 1024

RECORD_LIMIT = 8 * 1024 * 1024
MAX_RESPONSE_BYTES = 8 * 1024 * 1024


class Failure(Exception):
    """A refusal with a stable code, rather than a Python traceback."""

    def __init__(self, code, message):
        super().__init__(message)
        self.code = code
        self.message = message


def digest(data):
    return hashlib.sha256(data).hexdigest()


def request_path(q):
    """Absolute path for one request, with `~` expanded on the remote side."""
    value = q.get('path') or '.'
    if type(value) is not str:
        raise Failure('CONFIG_INVALID', 'path must be a string')
    if len(value) >= 2 and value[0] == value[-1] and value[0] in "'\"":
        raise Failure('CONFIG_INVALID',
                      'path includes outer shell quotes; quote the argument without making quotes part of the path')
    return os.path.abspath(os.path.expanduser(value))


def check_capacity(q):
    """The caller's response budget, validated before anything is read."""
    cap = integer(q, 'max_bytes', 0, 'CONFIG_INVALID')
    if cap <= 0 or cap > MAX_RESPONSE_BYTES:
        raise Failure('CONFIG_INVALID', 'max_bytes must be between 1 and %d' %
                      MAX_RESPONSE_BYTES)
    return cap


def integer(obj, key, default=None, code='CONFIG_INVALID'):
    """Read one JSON integer without accepting bool, float or numeric text."""
    value = obj.get(key, default)
    if type(value) is not int:
        raise Failure(code, '%s must be an integer' % key)
    return value


def resolve(q, path):
    """The real destination of a destructive sync, after symlinks are followed.

    `fs sync --delete` refuses broad destinations from the *text* the user typed
    (fileops.RejectSyncTarget), which cannot see that `/srv/app` is a symlink
    to `/`. This is the check on what the path actually names, and it runs only when
    the sync will really prune: copying files *into* a top-level directory is a
    legal thing to ask for.
    """
    actual = os.path.realpath(path)
    if not q.get('delete'):
        return {'path': actual}
    if actual == '/':
        raise Failure('SYNC_REJECTED', 'refusing to prune the filesystem root')
    if actual == os.path.expanduser('~'):
        raise Failure('SYNC_REJECTED', 'refusing to prune an entire home directory')
    if actual.count('/') < 2:
        raise Failure('SYNC_REJECTED', 'refusing to prune a top-level directory')
    return {'path': actual}


def clip_text(text, budget):
    """Clip to a byte budget without cutting a rune in half."""
    if budget <= 0:
        return '', True
    raw = text.encode('utf-8')
    if len(raw) <= budget:
        return text, False
    return raw[:budget].decode('utf-8', 'ignore'), True


def read_record(pipe, cap):
    """Read one complete line without holding more than RECORD_LIMIT bytes."""
    chunks, used = [], 0
    step = max(cap + 1, 65536)
    while used < RECORD_LIMIT:
        chunk = pipe.readline(min(step, RECORD_LIMIT - used))
        if not chunk:
            break
        chunks.append(chunk)
        used += len(chunk)
        if chunk.endswith(b'\n'):
            break
    return b''.join(chunks), used < RECORD_LIMIT


def read(q, path):
    """A bounded slice of a text file, plus the hash of the *whole* file.

    The hash has to cover everything, so the file is scanned once and sliced on
    the second pass; an external edit between the two passes is reported as a
    conflict instead of returning half-new content.
    """
    start = integer(q, 'start', 1)
    count = integer(q, 'lines', 200)
    if start < 1 or count < 1:
        raise Failure('CONFIG_INVALID', 'start and lines must be positive')
    cap = check_capacity(q)

    h, rows, used, total, truncated = hashlib.sha256(), [], 0, 0, False
    with open_file(path) as f:
        before = os.fstat(f.fileno())
        last = b''
        while True:
            chunk = f.read(65536)
            if not chunk:
                break
            h.update(chunk)
            total += chunk.count(b'\n')
            last = chunk[-1:]
        if last and last != b'\n':
            total += 1
        f.seek(0)
        index = 0
        while True:
            # Whole lines only, even when one is longer than the budget: a partial
            # read would shift every later line number against `--start`.
            raw, complete = read_record(f, cap)
            if not raw:
                break
            index += 1
            if index < start:
                continue
            if len(rows) >= count:
                # The page is full and there is more file after it: that is not
                # truncation, and `total_lines` already says how much is left.
                break
            if not complete:
                # A single line beyond RECORD_LIMIT: stop rather than return half
                # of it, and say so — `--max-bytes` cannot fix this, only `--start`
                # can page past the monster line.
                truncated = True
                break
            # Never cut mid-line: a page of `fs read` is whole lines, and a line
            # too big for the budget stops the slice instead. `--lines` and
            # `--start` page past it; `--max-bytes` raises the budget.
            if len(raw) > cap - used:
                truncated = True
                break
            rows.append(raw.decode('utf-8'))
            used += len(raw)
        after = os.fstat(f.fileno())
        if (before.st_size, before.st_mtime_ns, before.st_ctime_ns) != \
                (after.st_size, after.st_mtime_ns, after.st_ctime_ns):
            raise Failure('FILE_CONFLICT', 'file changed while reading')
    return {'path': path, 'sha256': h.hexdigest(), 'content': ''.join(rows), 'start': start,
            'lines': len(rows), 'total_lines': total, 'truncated': truncated}


@contextlib.contextmanager
def open_file(path):
    """Open a regular file, refusing anything a symlink or device could hide."""
    try:
        f = open(path, 'rb')
    except FileNotFoundError:
        raise Failure('FILE_NOT_FOUND', 'no such file: %s' % path)
    except IsADirectoryError:
        raise Failure('INVALID_TARGET', 'a directory is not a file: %s' % path)
    except PermissionError:
        raise Failure('INVALID_TARGET', 'cannot read %s' % path)
    try:
        if not stat.S_ISREG(os.fstat(f.fileno()).st_mode):
            raise Failure('INVALID_TARGET', 'not a regular file: %s' % path)
        yield f
    finally:
        f.close()


def requested_mode(q):
    """The permission the caller asked for, or None to keep the file's own.

    A new file is created private (0600) by default, because writing a file nobody
    asked to share should not make it readable by the world on a multi-user host.
    `--mode` overrides that — for a replacement too, since naming a permission is
    an explicit request, not a hint. Setuid and sticky bits are accepted because
    they are ordinary permissions; rhost does not decide to silently drop a mode
    the caller typed.
    """
    value = q.get('file_mode')
    if value in (None, ''):
        return None
    if type(value) is not str or len(value) not in (3, 4) or any(c not in '01234567' for c in value):
        raise Failure('CONFIG_INVALID', 'file_mode must be an octal permission such as 0644')
    mode = int(value, 8)
    if mode > 0o7777:
        raise Failure('CONFIG_INVALID', 'file_mode must be between 0000 and 07777')
    return mode


def write_or_patch(q, path):
    """Compare-and-swap write: hash-checked, locked, atomically replaced.

    Three separate guarantees, deliberately:
      * the parent-directory lock serialises *rhost* writers (it cannot lock an
        unrelated editor, so this is not a filesystem-wide compare-and-swap);
      * `if_hash` has to match the current content before *and* after the copy,
        so a concurrent edit surfaces as FILE_CONFLICT instead of being lost;
      * the replacement is a same-directory rename (or `link` when creating), so
        a reader never sees a half-written file, and deleting the target is never
        used as a fallback for a rename that did not happen.
    """
    op = q['op']
    parent = os.path.dirname(path)
    if op == 'write' and q.get('parents'):
        os.makedirs(parent, mode=0o700, exist_ok=True)
    if not os.path.exists(parent):
        raise Failure('FILE_NOT_FOUND', 'no such parent directory: %s' % parent)
    if not os.path.isdir(parent):
        raise Failure('INVALID_TARGET', 'parent is not a directory: %s' % parent)
    with contextlib.ExitStack() as stack:
        dfd = os.open(parent, os.O_RDONLY | os.O_DIRECTORY)
        stack.callback(os.close, dfd)
        fcntl.flock(dfd, fcntl.LOCK_EX)

        if os.path.islink(path):
            raise Failure('INVALID_TARGET', 'refusing to write through a symbolic link: %s' % path)
        exists = os.path.exists(path)
        old, mode = b'', 0o600
        if exists:
            st = os.stat(path)
            if not stat.S_ISREG(st.st_mode):
                raise Failure('INVALID_TARGET', 'target is not a regular file: %s' % path)
            if st.st_size > EDIT_LIMIT:
                raise Failure('FILE_TOO_LARGE', 'editing limit is %d bytes' % EDIT_LIMIT)
            with open(path, 'rb') as f:
                old = f.read()
            # An existing file keeps its own mode; a new one is created private.
            mode = stat.S_IMODE(st.st_mode)
        asked = requested_mode(q)
        if asked is not None:
            mode = asked

        expected = q.get('if_hash', '')
        if type(expected) is not str:
            raise Failure('CONFIG_INVALID', 'if_hash must be a string')
        if exists and not expected:
            raise Failure('HASH_REQUIRED', 'an existing file requires if_hash from fs read')
        if (exists and expected != digest(old)) or (not exists and (expected or op == 'patch')):
            raise Failure('FILE_CONFLICT', 'file changed or expected target is missing')

        if op == 'write':
            if type(q.get('content')) is not str:
                raise Failure('CONFIG_INVALID', 'content must be base64 text')
            try:
                data = base64.b64decode(q['content'], validate=True).decode('utf-8').encode('utf-8')
            except ValueError:
                raise Failure('CONFIG_INVALID', 'content must be valid base64 UTF-8 text')
        else:
            if 'edits' not in q:
                raise Failure('INVALID_PATCH', 'edits are required')
            data = apply_patch(old, q['edits'])
        if len(data) > EDIT_LIMIT:
            raise Failure('FILE_TOO_LARGE', 'editing limit is %d bytes' % EDIT_LIMIT)

        fd, tmp = tempfile.mkstemp(prefix='.rhost-write-', dir=parent)
        try:
            with os.fdopen(fd, 'wb') as f:
                f.write(data)
                f.flush()
                os.fsync(f.fileno())
                os.fchmod(f.fileno(), mode)
            if os.path.islink(path):
                raise Failure('FILE_CONFLICT', 'target became a symbolic link')
            if exists:
                with open(path, 'rb') as f:
                    if digest(f.read()) != expected:
                        raise Failure('FILE_CONFLICT', 'file changed before replacement')
                os.replace(tmp, path)
            else:
                try:
                    # `link`, not `rename`, so a concurrent creation of the same
                    # name fails loudly instead of replacing a file nobody read.
                    os.link(tmp, path)
                except FileExistsError:
                    raise Failure('FILE_CONFLICT', 'target was concurrently created')
            os.fsync(dfd)
        finally:
            if os.path.exists(tmp):
                os.unlink(tmp)

        with open(path, 'rb') as f:
            verified = digest(f.read(EDIT_LIMIT + 1))
        if verified != digest(data):
            raise Failure('FILE_CONFLICT', 'file changed during post-write verification')
        return {'path': path, 'sha256': verified, 'bytes': len(data)}


def apply_patch(old, edits):
    """Apply line-range replacements to the original bytes.

    Ranges are 1-based and inclusive, sorted, non-overlapping, and never beyond
    the file as it was read — a patch that would depend on an earlier edit's
    renumbering is rejected rather than silently mis-applied.
    """
    if type(edits) is not list:
        raise Failure('INVALID_PATCH', 'edits must be an array')
    for e in edits:
        if type(e) is not dict:
            raise Failure('INVALID_PATCH', 'each edit must be an object')
        if set(e) != {'start', 'end', 'text'}:
            raise Failure('INVALID_PATCH', 'each edit needs only start, end and text')
        integer(e, 'start', code='INVALID_PATCH')
        integer(e, 'end', code='INVALID_PATCH')
        if type(e['text']) is not str:
            raise Failure('INVALID_PATCH', 'edit text must be a string')
    lines = old.decode('utf-8').splitlines(keepends=True)
    ordered = sorted(edits, key=lambda e: e['start'])
    previous = 0
    for e in ordered:
        start, end = e['start'], e['end']
        if start <= previous or start < 1 or end < start or end > len(lines):
            raise Failure('INVALID_PATCH',
                          'edits must be non-overlapping 1-based inclusive ranges inside the file')
        previous = end
    for e in reversed(ordered):
        lines[e['start'] - 1:e['end']] = [e['text']]
    return ''.join(lines).encode('utf-8')


def run(q):
    if type(q) is not dict:
        raise Failure('CONFIG_INVALID', 'request must be a JSON object')
    op = q.get('op')
    path = request_path(q)
    if op == 'resolve':
        return resolve(q, path)
    if op == 'read':
        return read(q, path)
    if op in ('write', 'patch'):
        return write_or_patch(q, path)
    raise Failure('CONFIG_INVALID', 'unknown operation')


try:
    result = run(json.loads(sys.stdin.buffer.read(12 * 1024 * 1024)))
except Failure as f:
    result = {'error': f.code, 'message': f.message}
except UnicodeError:
    result = {'error': 'INVALID_TEXT', 'message': 'content must be UTF-8'}
except ValueError as e:
    # The JSON request itself did not parse, or a key held the wrong type.
    result = {'error': 'CONFIG_INVALID', 'message': str(e)}
except Exception as e:  # noqa: BLE001 - never leak a traceback to the CLI
    result = {'error': 'INTERNAL', 'message': '%s: %s' % (type(e).__name__, e)}
print(json.dumps(result, ensure_ascii=True))
