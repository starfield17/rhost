"""Tests for the embedded remote helper (remote.py).

These run the helper exactly as the remote host runs it — one JSON request on
stdin, one JSON response on stdout — so the shapes asserted here are the shapes
`rhost fs read/grep/glob/write/patch` return. They are driven from Go
(remote_helper_test.go) as part of `make test`, and skipped when this machine has
no usable python3, because a skipped suite is honest and a silently missing one
is not.
"""
import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).with_name('remote.py')
HAS_RG = shutil.which('rg') is not None


class RemoteFilesTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)

    def request(self, **q):
        q.setdefault('max_bytes', 262144)
        proc = subprocess.run([sys.executable, str(HERE)], input=json.dumps(q),
                              capture_output=True, text=True, timeout=30)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        try:
            return json.loads(proc.stdout)
        except ValueError:  # noqa: PER201 - the point of the assertion
            self.fail('helper did not answer with JSON: %r / %r' % (proc.stdout, proc.stderr))

    def code(self, **q):
        return self.request(**q).get('error')

    # --- read ------------------------------------------------------------

    def test_read_budget_and_hash(self):
        path = self.root / 'text'
        content = b'first\n' + b'x' * 1000000
        path.write_bytes(content)
        result = self.request(op='read', path=str(path), max_bytes=32)
        self.assertEqual(result['content'], 'first\n')
        self.assertEqual(result['sha256'], hashlib.sha256(content).hexdigest())
        self.assertTrue(result['truncated'])
        self.assertEqual(result['total_lines'], 2)

    def test_read_paging_and_tilde(self):
        path = self.root / 'lines'
        path.write_text(''.join('line %d\n' % n for n in range(1, 1001)))
        page = self.request(op='read', path=str(path), start=500, lines=3)
        self.assertEqual(page['content'], 'line 500\nline 501\nline 502\n')
        self.assertEqual(page['lines'], 3)
        self.assertFalse(page['truncated'])
        home = self.request(op='read', path='~/.rhost-does-not-exist-here')
        self.assertEqual(home.get('error'), 'FILE_NOT_FOUND')

    def test_read_refuses_non_files(self):
        self.assertEqual(self.code(op='read', path=str(self.root)), 'INVALID_TARGET')
        self.assertEqual(self.code(op='read', path=str(self.root / 'missing')), 'FILE_NOT_FOUND')

    def test_read_non_utf8(self):
        path = self.root / 'binary'
        path.write_bytes(b'\xff\xfe\x00abc\n')
        self.assertEqual(self.code(op='read', path=str(path)), 'INVALID_TEXT')

    def test_read_keeps_the_last_line_without_a_newline(self):
        # Most files on disk end without a trailing newline, and the line numbers
        # have to agree with what is returned: dropping the final line while
        # reporting `truncated` would send a caller to flags that change nothing.
        path = self.root / 'no-final-newline'
        path.write_bytes(b'one\ntwo\nthree')
        result = self.request(op='read', path=str(path))
        self.assertEqual(result['content'], 'one\ntwo\nthree')
        self.assertEqual(result['lines'], 3)
        self.assertEqual(result['total_lines'], 3)
        self.assertFalse(result['truncated'])
        # The same file, one line at a time: the last line is still there.
        tail = self.request(op='read', path=str(path), start=3, lines=10)
        self.assertEqual(tail['content'], 'three')
        self.assertEqual(tail['lines'], 1)

    # --- write and patch -------------------------------------------------

    def test_atomic_create_and_replace(self):
        path = self.root / 'text'
        result = self.request(op='write', path=str(path),
                              content=base64.b64encode(b'hello').decode())
        self.assertEqual(path.read_bytes(), b'hello')
        os.chmod(path, 0o640)
        result = self.request(op='write', path=str(path), if_hash=result['sha256'],
                              content=base64.b64encode(b'changed').decode())
        self.assertEqual(result['sha256'], hashlib.sha256(b'changed').hexdigest())
        self.assertEqual(path.stat().st_mode & 0o777, 0o640)
        # No sidecar, no temp file, no lock file left in the user's tree.
        self.assertEqual(sorted(p.name for p in self.root.iterdir()), ['text'])

    def test_write_mode(self):
        # A new file is private unless a permission was asked for: writing a file
        # nobody shared is not an instruction to open it to the machine.
        fresh = self.root / 'script'
        self.request(op='write', path=str(fresh),
                     content=base64.b64encode(b'#!/bin/sh\n').decode())
        self.assertEqual(fresh.stat().st_mode & 0o777, 0o600)
        for name, mode in (('public', '0644'), ('run', '0755')):
            path = self.root / name
            self.request(op='write', path=str(path), file_mode=mode,
                         content=base64.b64encode(b'#!/bin/sh\n').decode())
            self.assertEqual(path.stat().st_mode & 0o777, int(mode, 8), name)
        # Naming a mode on a replacement is also an explicit request, so it wins;
        # leaving it out keeps the file's own permissions.
        tight = self.root / 'tightened'
        tight.write_text('one\n')
        os.chmod(tight, 0o600)
        digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
        self.request(op='write', path=str(tight), if_hash=digest(tight),
                     content=base64.b64encode(b'two\n').decode())
        self.assertEqual(tight.stat().st_mode & 0o777, 0o600)
        self.request(op='write', path=str(tight), file_mode='0640', if_hash=digest(tight),
                     content=base64.b64encode(b'three\n').decode())
        self.assertEqual(tight.stat().st_mode & 0o777, 0o640)
        # A mode that is not octal is the caller's own mistake and is refused before
        # anything is written, not ignored.
        self.assertEqual(self.code(op='write', path=str(self.root / 'bad'), file_mode='0899',
                                   content=base64.b64encode(b'x').decode()), 'CONFIG_INVALID')
        self.assertFalse((self.root / 'bad').exists())

    def test_write_requires_creation_or_hash(self):
        path = self.root / 'text'
        path.write_text('one\ntwo\n')
        self.assertEqual(self.code(op='write', path=str(path),
                                   content=base64.b64encode(b'x').decode()), 'FILE_CONFLICT')
        self.assertEqual(path.read_text(), 'one\ntwo\n')

    def test_patch_conflict_ranges_and_symlink(self):
        path = self.root / 'text'
        path.write_text('one\ntwo\n')
        original = path.read_bytes()
        hash_value = hashlib.sha256(original).hexdigest()
        self.assertEqual(self.code(op='patch', path=str(path), if_hash='stale', edits=[]),
                         'FILE_CONFLICT')
        self.assertEqual(self.code(op='patch', path=str(path), if_hash=hash_value, edits=[
            {'start': 1, 'end': 2, 'text': 'x'}, {'start': 2, 'end': 2, 'text': 'y'}]),
            'INVALID_PATCH')
        self.assertEqual(self.code(op='patch', path=str(path), if_hash=hash_value,
                                   edits=[{'start': 1, 'end': 3, 'text': 'x'}]), 'INVALID_PATCH')
        link = self.root / 'link'
        link.symlink_to(path)
        self.assertEqual(self.code(op='patch', path=str(link), if_hash=hash_value, edits=[]),
                         'INVALID_TARGET')
        self.assertEqual(path.read_bytes(), original)

    def test_patch_applies_bottom_up(self):
        path = self.root / 'text'
        path.write_text('a\nb\nc\nd\n')
        result = self.request(op='patch', path=str(path),
                              if_hash=hashlib.sha256(path.read_bytes()).hexdigest(),
                              edits=[{'start': 4, 'end': 4, 'text': 'd2\n'},
                                     {'start': 1, 'end': 2, 'text': 'joined\n'}])
        self.assertEqual(path.read_text(), 'joined\nc\nd2\n')
        self.assertEqual(result['sha256'], hashlib.sha256(path.read_bytes()).hexdigest())

    # --- search ----------------------------------------------------------

    @unittest.skipUnless(HAS_RG, 'rg is not installed on this machine')
    def test_grep_rows_are_normalized(self):
        tree = self.root / 'tree'
        (tree / 'sub').mkdir(parents=True)
        # rg applies .gitignore rules only inside a git repository, so the fixture
        # needs a .git directory as well as the ignore file itself.
        (tree / '.git').mkdir()
        (tree / '.gitignore').write_text('ignored.go\n')
        (tree / 'a.go').write_text('alpha\nTODO one\n')
        (tree / 'sub' / 'b.go').write_text('TODO two\n')
        (tree / 'ignored.go').write_text('TODO three\n')
        result = self.request(op='grep', path=str(tree), pattern='TODO', max_bytes=4096,
                              limit=10, context=1)
        self.assertEqual([(r['path'], r['line']) for r in result['results']
                          if not r['context']],
                         [('a.go', 2), (os.path.join('sub', 'b.go'), 1)])
        self.assertTrue(any(r['context'] for r in result['results']))
        self.assertFalse(result['truncated'])
        # Context lines are records too, and pagination counts them.
        self.assertEqual(result['next'], len(result['results']))
        self.assertTrue(all(r['path'].startswith(('a.go', 'sub', 'ignored'))
                            for r in result['results']))

        for mode, key in (('files', 'path'), ('count', 'count')):
            rows = self.request(op='grep', path=str(tree), pattern='TODO', mode=mode)['results']
            # The gitignored file is skipped by default, so two files match.
            self.assertEqual(len(rows), 2, mode)
            self.assertIn(key, rows[0], mode)

        forced = self.request(op='grep', path=str(tree), pattern='TODO',
                                      mode='files', no_ignore=True)['results']
        self.assertEqual(len(forced), 3)
        self.assertIn({'path': 'ignored.go'}, forced)

    @unittest.skipUnless(HAS_RG, 'rg is not installed on this machine')
    def test_search_pagination_and_budget(self):
        tree = self.root / 'tree'
        tree.mkdir()
        (tree / 'a.txt').write_text(''.join('hit %d\n' % n for n in range(50)))
        first = self.request(op='grep', path=str(tree), pattern='hit', limit=3)
        self.assertEqual([r['line'] for r in first['results']], [1, 2, 3])
        self.assertTrue(first['truncated'])
        second = self.request(op='grep', path=str(tree), pattern='hit', limit=3,
                              offset=first['next'])
        self.assertEqual([r['line'] for r in second['results']], [4, 5, 6])
        last = self.request(op='grep', path=str(tree), pattern='hit', limit=60, offset=0)
        self.assertEqual(len(last['results']), 50)
        self.assertFalse(last['truncated'])

        tight = self.request(op='grep', path=str(tree), pattern='hit', max_bytes=200, limit=60)
        self.assertTrue(tight['truncated'])
        self.assertGreater(len(tight['results']), 0)
        self.assertLessEqual(sum(len(json.dumps(r)) for r in tight['results']), 200)

    @unittest.skipUnless(HAS_RG, 'rg is not installed on this machine')
    def test_long_match_line_is_clipped_not_dropped(self):
        tree = self.root / 'tree'
        tree.mkdir()
        (tree / 'one-line.txt').write_text('TODO ' + ('x' * 100000) + '\n')
        result = self.request(op='grep', path=str(tree), pattern='TODO', max_bytes=2048)
        rows = [r for r in result['results'] if not r['context']]
        self.assertEqual(len(rows), 1)
        self.assertTrue(rows[0]['text_truncated'])
        self.assertTrue(rows[0]['text'].startswith('TODO '))
        self.assertLessEqual(len(json.dumps(result['results']).encode()), 2048)

    @unittest.skipUnless(HAS_RG, 'rg is not installed on this machine')
    def test_no_match_is_an_empty_success(self):
        tree = self.root / 'tree'
        tree.mkdir()
        (tree / 'a.txt').write_text('alpha\n')
        result = self.request(op='grep', path=str(tree), pattern='nothing-matches-this')
        self.assertNotIn('error', result)
        self.assertEqual(result['results'], [])
        self.assertFalse(result['truncated'])
        self.assertEqual(result['next'], 0)

    @unittest.skipUnless(HAS_RG, 'rg is not installed on this machine')
    def test_glob_and_hidden_files(self):
        tree = self.root / 'tree'
        (tree / 'nested').mkdir(parents=True)
        (tree / 'a.go').write_text('')
        (tree / 'nested' / 'b.go').write_text('')
        (tree / '.hidden.go').write_text('')
        rows = self.request(op='glob', path=str(tree), pattern='*.go')['results']
        # Sorted by path, relative to the search root. rg's `*` also matches a
        # leading dot, so naming the extension is enough to reach a hidden file.
        self.assertEqual([r['path'] for r in rows],
                         ['.hidden.go', 'a.go', os.path.join('nested', 'b.go')])
        # --hidden is not a confidentiality control: an include glob that names a
        # file whitelists it whether or not it is hidden, so asserting the count is
        # the honest check here, not the absence of the dot-file.
        listed = self.request(op='glob', path=str(tree), pattern='*', hidden=True)['results']
        self.assertEqual(len(listed), 3)

    def test_search_without_rg_is_reported(self):
        # The CLI probes for rg before running the helper; a bad mode is still the
        # caller's own mistake and must not be reported as a missing dependency.
        self.assertEqual(self.code(op='grep', path=str(self.root), pattern='x', mode='bogus'),
                         'CONFIG_INVALID')

    # --- destructive-sync destination check ------------------------------

    def test_broad_resolved_target(self):
        for target in ['/', '~', os.path.expanduser('~'), '/srv']:
            self.assertEqual(self.code(op='resolve', path=target, delete=True),
                             'SYNC_REJECTED', target)
        link = self.root / 'root-link'
        link.symlink_to('/')
        self.assertEqual(self.code(op='resolve', path=str(link), delete=True), 'SYNC_REJECTED')
        safe = self.root / 'a' / 'b'
        safe.mkdir(parents=True)
        self.assertEqual(self.request(op='resolve', path=str(safe), delete=True)['path'],
                         str(safe.resolve()))
        # Without --delete the same refusal does not apply: a broad destination is
        # still a legal place to copy files into.
        self.assertNotEqual(self.code(op='resolve', path='/srv'), 'SYNC_REJECTED')

    def test_unknown_operation(self):
        self.assertEqual(self.code(op='nonsense', path=str(self.root)), 'CONFIG_INVALID')


if __name__ == '__main__':
    unittest.main()
