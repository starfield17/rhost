"""One-shot remote filesystem operations for `rhost fs`.

The program is embedded in the rhost binary and executed by a *remote* python3:
nothing is installed there, and a missing interpreter is reported as a
dependency failure instead (AGENTS.md §5). It reads one JSON request on stdin and
writes one JSON response on stdout, so no path or file content has to survive a
shell parser at either end.

Rules this file has to keep, because the CLI and its tests rely on them:

* `error` is always one of the codes the CLI knows; an unexpected exception
  becomes `INTERNAL`, never a guess;
* the response stays inside the request's own `max_bytes`, so a large file can
  never make the CLI drain an unbounded stream;
* nothing is written unless the request's hash precondition still holds;
* `resolve` intentionally follows symlinks because a destructive sync must know
  what its destination really names.
"""
import base64
import binascii
import contextlib
import errno
import fcntl
import hashlib
import json
import os
import stat
import sys
import tempfile

# Editing limit, in bytes, for a whole file: writes hold the new body in memory
# to hash and compare it. Reads are bounded by the caller's max_bytes instead.
EDIT_LIMIT = 8 * 1024 * 1024
RECORD_LIMIT = 8 * 1024 * 1024
MAX_RESPONSE_BYTES = 8 * 1024 * 1024


class Failure(Exception):
    """A refusal with a stable code, rather than a traceback."""

    def __init__(self, code, message):
        super().__init__(message)
        self.code = code
        self.message = message


def digest(data):
    return hashlib.sha256(data).hexdigest()


def number(q, key, default=None, code='CONFIG_INVALID'):
    """One JSON integer, rejecting bool, float and numeric text."""
    value = q.get(key, default)
    if type(value) is not int:
        raise Failure(code, '%s must be an integer' % key)
    return value


def request_path(q):
    """Absolute path for one request, with `~` expanded on the remote side."""
    value = q.get('path')
    if type(value) is not str or value == '':
        raise Failure('CONFIG_INVALID', 'path must be a nonempty string')
    if len(value) >= 2 and value[0] == value[-1] and value[0] in '\'"':
        raise Failure(
            'CONFIG_INVALID',
            'path includes outer shell quotes; quote the argument without making '
            'quotes part of the path')
    return os.path.abspath(os.path.expanduser(value))


def capacity(q):
    """The caller's response budget, validated before anything is read."""
    cap = number(q, 'max_bytes', 0)
    if cap <= 0 or cap > MAX_RESPONSE_BYTES:
        raise Failure('CONFIG_INVALID', 'max_bytes must be between 1 and %d' %
                      MAX_RESPONSE_BYTES)
    return cap


def opened(path):
    """Opens a regular file, refusing anything a symlink or device could hide."""
    try:
        handle = open(path, 'rb')
    except FileNotFoundError:
        raise Failure('FILE_NOT_FOUND', 'no such file: %s' % path)
    except IsADirectoryError:
        raise Failure('INVALID_TARGET', 'a directory is not a file: %s' % path)
    except PermissionError:
        raise Failure('INVALID_TARGET', 'cannot read %s' % path)
    except OSError as e:
        raise Failure('INVALID_TARGET', 'cannot read %s: %s' % (path, e.strerror))
    if not stat.S_ISREG(os.fstat(handle.fileno()).st_mode):
        handle.close()
        raise Failure('INVALID_TARGET', 'not a regular file: %s' % path)
    return handle


def read_record(pipe, cap):
    """Reads one whole line, without holding more than RECORD_LIMIT bytes.

    A line longer than the budget is not returned at all: `fs read` pages by
    whole lines, and half a line would shift every later line number.
    """
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
    complete = not chunks or chunks[-1].endswith(b'\n') or used < RECORD_LIMIT
    if not complete:
        # Consume the rest of this physical line without keeping it.
        chunk = pipe.readline(65536)
        complete = not chunk
        while chunk and not chunk.endswith(b'\n'):
            chunk = pipe.readline(65536)
    return b''.join(chunks), complete


def read(q, path):
    """A bounded slice of a text file, plus the SHA-256 of the *whole* file.

    The file is scanned once for the hash and sliced on a second pass; an edit
    between the passes is reported as a conflict rather than returning a page
    that matches neither the hash nor the file.
    """
    start = number(q, 'start', 1)
    count = number(q, 'lines', 200)
    if start < 1 or count < 1:
        raise Failure('CONFIG_INVALID', 'start and lines must be positive')
    cap = capacity(q)

    hasher, rows, used, total, truncated = hashlib.sha256(), [], 0, 0, False
    with opened(path) as handle:
        before = os.fstat(handle.fileno())
        last = b''
        while True:
            chunk = handle.read(65536)
            if not chunk:
                break
            hasher.update(chunk)
            total += chunk.count(b'\n')
            last = chunk[-1:]
        if last and last != b'\n':
            total += 1
        handle.seek(0)
        index = 0
        while True:
            raw, complete = read_record(handle, cap)
            if not raw:
                break
            index += 1
            if index < start:
                continue
            if len(rows) >= count:
                break
            if not complete or len(raw) > cap - used:
                truncated = True
                break
            rows.append(raw.decode('utf-8'))
            used += len(raw)
        after = os.fstat(handle.fileno())
        if (before.st_size, before.st_mtime_ns, before.st_ctime_ns) != \
                (after.st_size, after.st_mtime_ns, after.st_ctime_ns):
            raise Failure('FILE_CONFLICT', 'file changed while reading')
    return {'path': path, 'sha256': hasher.hexdigest(), 'content': ''.join(rows),
            'start': start, 'lines': len(rows), 'total_lines': total,
            'truncated': truncated}


def requested_mode(q):
    """The permission the caller named, or None to keep the file's own.

    A new file is created private by default: writing a file nobody asked to
    share must not make it world-readable on a multi-user host. A named mode is
    an explicit request and applies to a replacement too.
    """
    value = q.get('file_mode')
    if value in (None, ''):
        return None
    if type(value) is not str or len(value) not in (3, 4) or \
            any(c not in '01234567' for c in value):
        raise Failure('CONFIG_INVALID', 'file_mode must be an octal permission such as 0644')
    mode = int(value, 8)
    if mode > 0o7777:
        raise Failure('CONFIG_INVALID', 'file_mode must be between 0000 and 07777')
    return mode


def new_body(q, op, old):
    if op == 'write':
        content = q.get('content')
        if type(content) is not str:
            raise Failure('CONFIG_INVALID', 'content must be base64 text')
        try:
            raw = base64.b64decode(content, validate=True)
        except (binascii.Error, ValueError):
            raise Failure('CONFIG_INVALID', 'content must be valid base64 text')
        try:
            # This is a text-editing surface: the body must be UTF-8 like every
            # page `fs read` returns (FS-003).
            return raw.decode('utf-8').encode('utf-8')
        except UnicodeDecodeError:
            raise Failure('INVALID_TEXT', 'content must be UTF-8')
    if op == 'patch':
        if 'edits' not in q:
            raise Failure('INVALID_PATCH', 'edits are required')
        return apply_patch(old, q['edits'])
    raise Failure('CONFIG_INVALID', 'unknown operation')


def write_or_patch(q, path):
    """Compare-and-swap write: hash-checked, locked, atomically replaced.

    Three separate guarantees:
      * the parent-directory lock serialises rhost writers (it cannot lock an
        unrelated editor, so this is not a filesystem-wide compare-and-swap);
      * `if_hash` has to match the current content before *and* after the copy,
        so a concurrent edit surfaces as FILE_CONFLICT instead of being lost;
      * the replacement is a same-directory rename (or `link` when creating), so
        a reader never sees half a file, and deleting the target is never used
        as a fallback for a rename that did not happen.
    """
    op = q['op']
    parent = os.path.dirname(path)
    if op == 'write' and q.get('parents'):
        os.makedirs(parent, mode=0o700, exist_ok=True)
    if not os.path.isdir(parent):
        raise Failure('FILE_NOT_FOUND', 'no such parent directory: %s' % parent)
    with contextlib.ExitStack() as stack:
        directory = os.open(parent, os.O_RDONLY | os.O_DIRECTORY)
        stack.callback(os.close, directory)
        fcntl.flock(directory, fcntl.LOCK_EX)

        if os.path.islink(path):
            raise Failure('INVALID_TARGET', 'refusing to write through a symbolic link: %s' % path)
        exists = os.path.exists(path)
        old, mode = b'', 0o600
        if exists:
            info = os.stat(path)
            if not stat.S_ISREG(info.st_mode):
                raise Failure('INVALID_TARGET', 'target is not a regular file: %s' % path)
            if info.st_size > EDIT_LIMIT:
                raise Failure('FILE_TOO_LARGE', 'editing limit is %d bytes' % EDIT_LIMIT)
            with open(path, 'rb') as handle:
                old = handle.read()
            mode = stat.S_IMODE(info.st_mode)
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

        data = new_body(q, op, old)
        if len(data) > EDIT_LIMIT:
            raise Failure('FILE_TOO_LARGE', 'editing limit is %d bytes' % EDIT_LIMIT)

        descriptor, temporary = tempfile.mkstemp(prefix='.rhost-write-', dir=parent)
        try:
            with os.fdopen(descriptor, 'wb') as handle:
                handle.write(data)
                handle.flush()
                os.fsync(handle.fileno())
                os.fchmod(handle.fileno(), mode)
            if os.path.islink(path):
                raise Failure('FILE_CONFLICT', 'target became a symbolic link')
            if exists:
                with open(path, 'rb') as handle:
                    if digest(handle.read()) != expected:
                        raise Failure('FILE_CONFLICT', 'file changed before replacement')
                os.replace(temporary, path)
            else:
                try:
                    # `link`, not `rename`: a concurrent creation of the same
                    # name fails loudly instead of replacing a file nobody read.
                    os.link(temporary, path)
                except FileExistsError:
                    raise Failure('FILE_CONFLICT', 'target was concurrently created')
            os.fsync(directory)
        finally:
            if os.path.exists(temporary):
                os.unlink(temporary)

        with open(path, 'rb') as handle:
            verified = digest(handle.read(EDIT_LIMIT + 1))
        if verified != digest(data):
            raise Failure('FILE_CONFLICT', 'file changed during post-write verification')
        return {'path': path, 'sha256': verified, 'bytes': len(data)}


def apply_patch(old, edits):
    """Applies line-range replacements to the original bytes.

    Ranges are 1-based, inclusive, sorted and non-overlapping, and never beyond
    the file as it was read: a patch that would depend on an earlier edit's
    renumbering is refused rather than silently mis-applied.
    """
    if type(edits) is not list or not edits:
        raise Failure('INVALID_PATCH', 'edits must be a nonempty array')
    for edit in edits:
        if type(edit) is not dict:
            raise Failure('INVALID_PATCH', 'each edit must be an object')
        if set(edit) != {'start', 'end', 'text'}:
            raise Failure('INVALID_PATCH', 'each edit needs only start, end and text')
        number(edit, 'start', code='INVALID_PATCH')
        number(edit, 'end', code='INVALID_PATCH')
        if type(edit['text']) is not str:
            raise Failure('INVALID_PATCH', 'edit text must be a string')
    try:
        text = old.decode('utf-8')
    except UnicodeDecodeError:
        raise Failure('INVALID_TEXT', 'file is not UTF-8 text')
    # Only LF separates lines, matching read(): CR is literal content, and a
    # final line without a newline is preserved as it is.
    parts = text.split('\n')
    lines = [part + '\n' for part in parts[:-1]]
    if parts[-1]:
        lines.append(parts[-1])
    previous = 0
    for edit in sorted(edits, key=lambda e: e['start']):
        start, end = edit['start'], edit['end']
        if start <= previous or start < 1 or end < start or end > len(lines):
            raise Failure(
                'INVALID_PATCH',
                'edits must be non-overlapping 1-based inclusive ranges inside the file')
        previous = end
    for edit in sorted(edits, key=lambda e: e['start'], reverse=True):
        lines[edit['start'] - 1:edit['end']] = [edit['text']]
    return ''.join(lines).encode('utf-8')


def resolve(q, path):
    """The real destination of a destructive sync, after symlinks are followed.

    `fs sync --delete` refuses broad destinations from the text the user typed,
    which cannot see that `/srv/app` is a symlink to `/`. This is the check on
    what the path actually names, and it runs only when the sync will prune.
    """
    actual = os.path.realpath(path)
    if not q.get('delete'):
        return {'path': actual}
    if actual == '/':
        raise Failure('SYNC_REJECTED', 'refusing to prune the filesystem root')
    if actual == os.path.realpath(os.path.expanduser('~')):
        raise Failure('SYNC_REJECTED', 'refusing to prune an entire home directory')
    if actual.count('/') < 2:
        raise Failure('SYNC_REJECTED', 'refusing to prune a top-level directory')
    return {'path': actual}


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
except Failure as failure:
    result = {'error': failure.code, 'message': failure.message}
except UnicodeError:
    result = {'error': 'INVALID_TEXT', 'message': 'content must be UTF-8'}
except ValueError as error:
    result = {'error': 'CONFIG_INVALID', 'message': str(error)}
except OSError as error:
    code = 'FILE_NOT_FOUND' if error.errno == errno.ENOENT else 'INTERNAL'
    result = {'error': code, 'message': 'remote helper: %s' % error}
except Exception as error:  # noqa: BLE001 - never leak a traceback to the CLI
    result = {'error': 'INTERNAL', 'message': '%s: %s' % (type(error).__name__, error)}
print(json.dumps(result, ensure_ascii=True))
