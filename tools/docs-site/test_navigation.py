import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('docs_site', Path(__file__).with_name('site.py'))
docs_site = importlib.util.module_from_spec(spec)
spec.loader.exec_module(docs_site)


class NavigationTest(unittest.TestCase):
    def test_readme_order_and_omitted_page(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manual = root / 'docs/manual'
            manual.mkdir(parents=True)
            (manual / 'README.md').write_text('# Manual\n[B](b.md)\n[A](a.md)\n')
            for name in ['a.md', 'b.md']:
                (manual / name).write_text('# A page\n')
            with patch.object(docs_site, 'ROOT', root):
                self.assertEqual(['b.md', 'a.md'], [p.name for p in docs_site.ordered_pages('manual')])
                (manual / 'new.md').write_text('# New\n')
                with self.assertRaisesRegex(ValueError, 'missing=.*new.md'):
                    docs_site.ordered_pages('manual')
