from contextlib import redirect_stdout
import io
from pathlib import Path
import tempfile
import unittest

from links import check


class LinksTest(unittest.TestCase):
    def inspect(self, html, files=()):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'index.html').write_text(html)
            for name, content in files:
                target = root / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(content)
            with redirect_stdout(io.StringIO()):
                return check(root)

    def test_encoded_fragment_query_and_assets(self):
        self.assertEqual([], self.inspect(
            '<a href="page%20one.html?q=yes#83-%CE%BB">ok</a><script src="app.js"></script>',
            [('page one.html', '<h2 id="83-λ">heading</h2>'), ('app.js', '')]))

    def test_missing_page(self):
        failures = self.inspect('<a href="absent.html">bad</a>')
        self.assertEqual(['index.html: absent.html: missing file'], failures)

    def test_missing_anchor(self):
        failures = self.inspect('<a href="#absent">bad</a>')
        self.assertEqual(['index.html: #absent: missing anchor'], failures)

    def test_escaped_path(self):
        failures = self.inspect('<a href="%2e%2e/outside.html">bad</a>')
        self.assertEqual(['index.html: %2e%2e/outside.html: path escapes site root'], failures)

    def test_external_and_root_directory(self):
        self.assertEqual([], self.inspect(
            '<a href="https://example.com/no">external</a><a href="/sub/">directory</a>',
            [('sub/index.html', '')]))

    def test_missing_asset(self):
        self.assertEqual(['index.html: missing.png: missing file'],
                         self.inspect('<img src="missing.png">'))
