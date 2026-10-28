from pathlib import Path
import tempfile
import unittest

from navigation import expand, ordered_pages


class NavigationTest(unittest.TestCase):
    def test_readme_order_and_omitted_page(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)
            manual = source / 'manual'
            manual.mkdir()
            (manual / 'README.md').write_text('# Manual\n[B](b.md)\n[A](a.md)\n')
            for name in ['a.md', 'b.md']:
                (manual / name).write_text('# A page\n')
            self.assertEqual(['b.md', 'a.md'], [p.name for p in ordered_pages(source, 'manual')])
            (manual / 'new.md').write_text('# New\n')
            with self.assertRaisesRegex(ValueError, 'missing=.*new.md'):
                ordered_pages(source, 'manual')

    def test_chapters_preserve_source_and_relative_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manual = root / 'docs/manual'
            manual.mkdir(parents=True)
            content = '# Child\n[Parent](README.md#manual)\n'
            (manual / 'README.md').write_text('# Manual\n[Child](child.md)\n')
            (manual / 'child.md').write_text(content)
            parent = {'name': 'Manual', 'path': 'manual/README.md',
                      'number': [2], 'parent_names': [], 'sub_items': []}
            items = [{'Chapter': parent}]
            for section, heading in [('spec', '## Parts\n'), ('design', '## 7. Files\n')]:
                path = root / 'docs' / section
                path.mkdir()
                (path / 'README.md').write_text('# Section\n' + heading)
                items.append({'Chapter': {'name': section, 'path': f'{section}/README.md',
                                          'number': [3], 'parent_names': [], 'sub_items': []}})
            items.extend(['Separator', {'PartTitle': 'Appendix'}])
            book = {'items': items}
            result = expand({'root': str(root), 'config': {'book': {'src': 'docs'}}}, book)
            child = result['items'][0]['Chapter']['sub_items'][0]['Chapter']
            self.assertEqual(content, child['content'])
            self.assertEqual('manual/child.md', child['path'])
            self.assertEqual(child['path'], child['source_path'])
            self.assertEqual(['Manual'], child['parent_names'])
            self.assertEqual([2, 1], child['number'])
            self.assertEqual(['Separator', {'PartTitle': 'Appendix'}], result['items'][-2:])

    def test_summary_cannot_omit_a_documentation_section(self):
        with self.assertRaisesRegex(ValueError, 'SUMMARY missing section roots:'):
            expand({'root': '.', 'config': {'book': {'src': 'docs'}}}, {'items': []})
