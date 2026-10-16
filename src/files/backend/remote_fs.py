"""One-shot remote filesystem operations for `rhost fs`.

Embedded in the rhost binary and executed by remote python3; nothing is installed
there (AGENTS.md §5). Reads one JSON request on stdin and writes one JSON response
on stdout.
"""
import base64
import binascii
import errno
import fcntl
import hashlib
import json
import os
import stat
import sys

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


def target_parts(path):
    """The parent directory and exact basename to operate on together."""
    parent, name = os.path.split(path)
    if not name:
        raise Failure('INVALID_TARGET', 'the filesystem root is not a file')
    return parent, name


def traversal_failure(exc, name):
    if exc.errno == errno.ENOENT:
        return Failure('FILE_NOT_FOUND', 'no such parent directory: %s' % name)
    if exc.errno == errno.ELOOP:
        return Failure('INVALID_TARGET', 'refusing a symbolic link: %s' % name)
    if exc.errno in (errno.ENOTDIR, errno.EISDIR):
        return Failure('INVALID_TARGET', 'not a traversable directory: %s' % name)
    if exc.errno in (errno.EACCES, errno.EPERM):
        return Failure('INVALID_TARGET', 'cannot traverse %s' % name)
    return Failure('INTERNAL', 'cannot open %s: %s' % (name, exc.strerror))


def open_directory(path, parents=False):
    fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in [p for p in path.split('/') if p and p != '.']:
            try:
                child = os.open(part, os.O_RDONLY | os.O_NOFOLLOW | os.O_DIRECTORY, dir_fd=fd)
            except OSError as exc:
                if not parents or exc.errno != errno.ENOENT:
                    raise traversal_failure(exc, part)
                try:
                    os.mkdir(part, mode=0o700, dir_fd=fd)
                except FileExistsError:
                    pass
                try:
                    child = os.open(part, os.O_RDONLY | os.O_NOFOLLOW | os.O_DIRECTORY, dir_fd=fd)
                except OSError as exc:
                    raise traversal_failure(exc, part)
            os.close(fd)
            fd = child
    except BaseException:
        os.close(fd)
        raise
    return fd


def target_failure(path, exc, changed=False):
    if changed and exc.errno == errno.ENOENT:
        return Failure('FILE_CONFLICT', 'target disappeared before replacement')
    if changed and exc.errno == errno.ELOOP:
        return Failure('FILE_CONFLICT', 'target became a symbolic link')
    if exc.errno == errno.ENOENT:
        return Failure('FILE_NOT_FOUND', 'no such file: %s' % path)
    if exc.errno in (errno.ELOOP, errno.ENOTDIR):
        return Failure('INVALID_TARGET', 'refusing a symbolic link or non-directory: %s' % path)
    if exc.errno == errno.EISDIR:
        return Failure('INVALID_TARGET', 'a directory is not a file: %s' % path)
    if exc.errno in (errno.EACCES, errno.EPERM):
        return Failure('INVALID_TARGET', 'cannot access %s' % path)
    return Failure('INTERNAL', 'cannot access %s: %s' % (path, exc.strerror))


def open_regular(path, directory, name, flags, changed=False):
    try:
        fd = os.open(name, flags | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
    except OSError as exc:
        raise target_failure(path, exc, changed)
    if not stat.S_ISREG(os.fstat(fd).st_mode):
        os.close(fd)
        raise Failure('INVALID_TARGET', 'not a regular file: %s' % path)
    return fd


def opened(path):
    parent, name = target_parts(path)
    directory = open_directory(parent)
    try:
        return os.fdopen(open_regular(path, directory, name, os.O_RDONLY), 'rb')
    finally:
        os.close(directory)


def temp_in_directory(directory):
    for _ in range(100):
        name = '.rhost-write-%s' % os.urandom(8).hex()
        try:
            return os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=directory), name
        except FileExistsError:
            continue
    raise Failure('INTERNAL', 'could not allocate a unique temporary file')


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
    """Compare-and-swap write: hash-checked, locked, atomically replaced."""
    op = q['op']
    parent, name = target_parts(path)
    directory = open_directory(parent, op == 'write' and q.get('parents'))
    try:
        fcntl.flock(directory, fcntl.LOCK_EX)
        try:
            info = os.stat(name, dir_fd=directory, follow_symlinks=False)
        except FileNotFoundError:
            info = None
        except OSError as exc:
            raise target_failure(path, exc)

        old, mode = b'', 0o600
        if info is not None:
            if stat.S_ISLNK(info.st_mode):
                raise Failure('INVALID_TARGET', 'refusing to write through a symbolic link: %s' % path)
            if not stat.S_ISREG(info.st_mode):
                raise Failure('INVALID_TARGET', 'target is not a regular file: %s' % path)
            if info.st_size > EDIT_LIMIT:
                raise Failure('FILE_TOO_LARGE', 'editing limit is %d bytes' % EDIT_LIMIT)
            fd = open_regular(path, directory, name, os.O_RDONLY, changed=True)
            with os.fdopen(fd, 'rb') as handle:
                old = handle.read()
            mode = stat.S_IMODE(info.st_mode)
        asked = requested_mode(q)
        if asked is not None:
            mode = asked

        expected = q.get('if_hash', '')
        if type(expected) is not str:
            raise Failure('CONFIG_INVALID', 'if_hash must be a string')
        if info is not None and not expected:
            raise Failure('HASH_REQUIRED', 'an existing file requires if_hash from fs read')
        if (info is not None and expected != digest(old)) or \
                (info is None and (expected or op == 'patch')):
            raise Failure('FILE_CONFLICT', 'file changed or expected target is missing')

        data = new_body(q, op, old)
        if len(data) > EDIT_LIMIT:
            raise Failure('FILE_TOO_LARGE', 'editing limit is %d bytes' % EDIT_LIMIT)

        fd, temporary = temp_in_directory(directory)
        try:
            with os.fdopen(fd, 'wb') as handle:
                handle.write(data)
                handle.flush()
                os.fsync(handle.fileno())
                os.fchmod(handle.fileno(), mode)
            if info is not None:
                fd = open_regular(path, directory, name, os.O_RDONLY, changed=True)
                with os.fdopen(fd, 'rb') as handle:
                    if digest(handle.read()) != expected:
                        raise Failure('FILE_CONFLICT', 'file changed before replacement')
                os.rename(temporary, name, src_dir_fd=directory, dst_dir_fd=directory)
            else:
                try:
                    # `link`, not `rename`: a concurrent creation of the same
                    # name fails loudly instead of replacing a file nobody read.
                    os.link(temporary, name, src_dir_fd=directory, dst_dir_fd=directory)
                except FileExistsError:
                    raise Failure('FILE_CONFLICT', 'target was concurrently created')
            os.fsync(directory)
        finally:
            try:
                os.unlink(temporary, dir_fd=directory)
            except FileNotFoundError:
                pass

        fd = open_regular(path, directory, name, os.O_RDONLY, changed=True)
        with os.fdopen(fd, 'rb') as handle:
            verified = digest(handle.read(EDIT_LIMIT + 1))
        if verified != digest(data):
            raise Failure('FILE_CONFLICT', 'file changed during post-write verification')
        return {'path': path, 'sha256': verified, 'bytes': len(data)}
    finally:
        os.close(directory)


def apply_patch(old, edits):
    """Applies line-range or exact-text replacements to the original bytes.

    Line ranges are 1-based, inclusive, sorted and non-overlapping, and never
    beyond the file as it was read. Exact-text replacements require `old` to
    appear exactly once in the file and edits must not overlap. Mixing line-range
    and exact-text edits in one patch is refused.
    """
    if type(edits) is not list or not edits:
        raise Failure('INVALID_PATCH', 'edits must be a nonempty array')
    for edit in edits:
        if type(edit) is not dict:
            raise Failure('INVALID_PATCH', 'each edit must be an object')
        keys = set(edit)
        if keys == {'start', 'end', 'text'}:
            number(edit, 'start', code='INVALID_PATCH')
            number(edit, 'end', code='INVALID_PATCH')
            if type(edit['text']) is not str:
                raise Failure('INVALID_PATCH', 'edit text must be a string')
        elif keys == {'old', 'new'}:
            if type(edit['old']) is not str or type(edit['new']) is not str:
                raise Failure('INVALID_PATCH', 'old and new must be strings')
            if not edit['old']:
                raise Failure('INVALID_PATCH', 'old text must not be empty')
        else:
            raise Failure(
                'INVALID_PATCH',
                'each edit needs either (start, end, text) or (old, new)')

    has_range = any('start' in edit for edit in edits)
    has_replace = any('old' in edit for edit in edits)
    if has_range and has_replace:
        raise Failure('INVALID_PATCH', 'cannot mix line-range and text-replacement edits')

    try:
        text = old.decode('utf-8')
    except UnicodeDecodeError:
        raise Failure('INVALID_TEXT', 'file is not UTF-8 text')

    if has_range:
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

    spans = []
    for edit in edits:
        target = edit['old']
        count = text.count(target)
        if count != 1:
            raise Failure(
                'INVALID_PATCH',
                'old text must appear exactly once in file, found %d' % count)
        start_idx = text.find(target)
        end_idx = start_idx + len(target)
        spans.append((start_idx, end_idx, edit['new']))
    spans.sort(key=lambda s: s[0])
    for i in range(len(spans) - 1):
        if spans[i][1] > spans[i + 1][0]:
            raise Failure('INVALID_PATCH', 'replacement edits must not overlap')
    for start_idx, end_idx, new_text in reversed(spans):
        text = text[:start_idx] + new_text + text[end_idx:]
    return text.encode('utf-8')


def resolve(q, path):
    """The real destination of a destructive sync, after symlinks are followed.

    `fs sync --delete` refuses broad destinations from the text the user typed,
    which cannot see that `/srv/app` is a symlink to `/`. This is the check on
    what the path actually names, and it runs only when the sync will prune.
    Editing primitives use the opposite rule: they name an exact entry, never
    the alias that happens to reach it.
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
