"""One-shot remote file operations, used by `rhost fs read/write/patch/grep/glob`.

The program is embedded into the rhost binary and executed by a *remote* python3
(AGENTS.md §5: nothing is installed here, a missing interpreter is reported as a
dependency failure instead). It reads one JSON request on stdin and writes one
JSON response on stdout, so no path, pattern or file content ever has to survive
a shell parser, and the CLI needs only one transport (docs/ARCHITECTURE.md §28).

Stability rules this file has to keep:

* `error` is always one of the codes in internal/errs; an unexpected exception
  becomes `INTERNAL`, never a guess;
* a response is bounded by the request's own `max_bytes`, so a large file or a
  large search can never make the CLI drain an unbounded stream;
* nothing is written unless the request's hash precondition still matches.
"""
import base64
import contextlib
import fcntl
import hashlib
import json
import os
import stat
import subprocess
import sys
import tempfile

# Editing limit, in bytes, for a whole file. Reads and searches are bounded by
# the caller's max_bytes instead; this bounds *writes*, where the whole new body
# has to be held in memory to be hashed and compared.
EDIT_LIMIT = 8 * 1024 * 1024

# Ceiling for one streamed search record. rg has already buffered the line, so
# reading it is not the risk; holding a multi-gigabyte line in this process is.
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


def read_record(pipe, cap):
    """One complete line from `pipe`, as (bytes, complete).

    A record ends at its newline, at the end of the stream, or at RECORD_LIMIT —
    whichever comes first, and only the last of those is incomplete. A file whose
    final line has no trailing newline is the ordinary case (most editors write
    one), so treating "no newline" as truncation would drop the last line of every
    such file and point the caller at flags that could never help.

    A line is read even when it is longer than the caller's budget — `rg` has
    already buffered it — but never longer than RECORD_LIMIT, so a match inside a
    2 GB single-line file cannot make the helper allocate 2 GB.
    """
    chunks, used = [], 0
    step = max(cap + 1, 65536)
    while used < RECORD_LIMIT:
        chunk = pipe.readline(min(step, RECORD_LIMIT - used))
        if not chunk:
            break  # end of stream: what was read is the whole record
        chunks.append(chunk)
        used += len(chunk)
        if chunk.endswith(b'\n'):
            break
    line = b''.join(chunks)
    return line, used < RECORD_LIMIT


def clip_text(text, budget):
    """Clip to a byte budget without cutting a rune in half."""
    if budget <= 0:
        return '', True
    raw = text.encode('utf-8')
    if len(raw) <= budget:
        return text, False
    return raw[:budget].decode('utf-8', 'ignore'), True


def records(pipe, op, mode, cap):
    """Yield one normalised result row per record `pipe` reports.

    `rg`'s own JSON is an event stream — `begin`/`match`/`end`/`stats`, with every
    field nested under `data`. Passing that through would put a tool's wire
    format into rhost's contract, and cost about four times the bytes of the
    matching text, so the search budget would buy less than it appears to. These
    rows are the stable shape instead.
    """
    per_record = max(cap // 8, 512)
    while True:
        raw, complete = read_record(pipe, cap)
        if not raw:
            return
        if not complete:
            raise Failure('SEARCH_FAILED',
                          'a single result exceeded %d bytes; narrow the pattern or raise max_bytes'
                          % RECORD_LIMIT)
        text = raw.decode('utf-8', 'replace').rstrip('\n')
        if op == 'grep' and mode == 'content':
            try:
                event = json.loads(text)
                kind = event.get('type')
                if kind not in ('match', 'context'):
                    continue
                data = event['data']
                row = {'path': relative(data['path']['text']), 'line': data['line_number'],
                       'context': kind == 'context'}
                body, clipped = clip_text(data['lines']['text'].rstrip('\n'), per_record)
                row['text'] = body
                if clipped:
                    row['text_truncated'] = True
                if kind == 'match' and data.get('column_number'):
                    row['column'] = data['column_number']
            except (ValueError, KeyError, TypeError):
                raise Failure('SEARCH_FAILED', 'cannot parse this search result: %s' % text[:200])
        elif op == 'grep' and mode == 'count':
            # `--count` prints `path:count`; the count is after the last colon, so
            # a filename containing one still parses.
            try:
                name, count = text.rsplit(':', 1)
                row = {'path': relative(name), 'count': int(count)}
            except ValueError:
                raise Failure('SEARCH_FAILED', 'cannot parse this search result: %s' % text[:200])
        else:
            row = {'path': relative(text)}
        yield row


def relative(name):
    """Paths are reported relative to the search root, without a leading `./`."""
    return name[2:] if name.startswith('./') else name


def search(q, path, op):
    """`rg`-backed grep/glob, capped while the results stream in.

    The budget is applied per record and in total, and the child is killed once it
    is spent, so a recursive search of a huge tree costs `max_bytes` rather than
    one whole tree. `truncated` says which happened, and `next` is the offset to
    pass back for the following page.
    """
    mode = q.get('mode') or 'content'
    if mode not in ('content', 'files', 'count'):
        raise Failure('CONFIG_INVALID', 'mode must be content, files or count')
    if not os.path.exists(path):
        raise Failure('FILE_NOT_FOUND', 'no such directory: %s' % path)
    if not os.path.isdir(path):
        raise Failure('INVALID_TARGET', 'search root is not a directory: %s' % path)
    limit = integer(q, 'limit', 100)
    offset = integer(q, 'offset', 0)
    if limit <= 0 or offset < 0:
        raise Failure('CONFIG_INVALID', 'limit must be positive and offset nonnegative')
    cap = check_capacity(q)
    if type(q.get('pattern')) is not str:
        raise Failure('CONFIG_INVALID', 'a pattern is required')

    args = ['rg']
    if q.get('hidden'):
        args.append('--hidden')
    if q.get('no_ignore'):
        args.append('--no-ignore')
    if op == 'glob':
        args += ['--files', '--sort', 'path', '-g', q['pattern']]
    else:
        args += ['--sort', 'path']
        if mode == 'content':
            args.append('--json')
        elif mode == 'files':
            args.append('--files-with-matches')
        else:
            args.append('--count')
        if q.get('ignore_case'):
            args.append('-i')
        if q.get('glob'):
            if type(q['glob']) is not str:
                raise Failure('CONFIG_INVALID', 'glob must be a string')
            args += ['-g', q['glob']]
        context = integer(q, 'context', 0)
        if context < 0:
            raise Failure('CONFIG_INVALID', 'context must be nonnegative')
        args += ['-C', str(context), '-e', q['pattern']]
    args += ['--', '.']

    rows, used, seen, truncated = [], 0, 0, False
    with tempfile.TemporaryFile() as errors:
        proc = subprocess.Popen(args, cwd=path, stdout=subprocess.PIPE, stderr=errors)
        try:
            for row in records(proc.stdout, op, mode, cap):
                # Counted for pagination, budgeted for the response: an offset
                # skips records, it does not skip bytes.
                if seen < offset:
                    seen += 1
                    continue
                cost = len(json.dumps(row))
                if len(rows) >= limit:
                    truncated = True
                    break
                if used + cost > cap:
                    # This record has been consumed but cannot fit even on an
                    # otherwise empty page. Advance past it so the cursor can
                    # never deadlock on the same oversized result.
                    seen += 1
                    truncated = True
                    break
                rows.append(row)
                used += cost
                seen += 1
            if truncated:
                proc.terminate()
            rc = proc.wait()
            # rg exits 1 for "no matches", which is an empty success, and 2 for a
            # real error. Anything else is the tool's own failure, quoted.
            if not truncated and rc not in (0, 1):
                errors.seek(0)
                raise Failure('SEARCH_FAILED', errors.read(4096).decode('utf-8', 'replace').strip())
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.wait()
            proc.stdout.close()
    return {'path': path, 'results': rows, 'truncated': truncated, 'next': seen}


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
    if op in ('grep', 'glob'):
        return search(q, path, op)
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
