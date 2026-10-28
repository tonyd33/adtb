import json
from pathlib import Path
import re
import sys


SECTIONS = ['manual', 'spec', 'design']


def ordered_pages(source, section):
    directory = source / section
    index = (directory / 'README.md').read_text()
    heading = {'spec': '## Parts\n', 'design': '## 7. Files\n'}.get(section)
    if heading:
        _, marker, index = index.partition(heading)
        if not marker:
            raise ValueError(f'{section} README missing navigation heading {heading.strip()!r}')
        index = index.split('\n## ', 1)[0]
    pages = []
    for target in re.findall(r'\]\(([^)#]+)(?:#[^)]*)?\)', index):
        path = (directory / target).resolve()
        if (path.is_relative_to(directory) and path.suffix == '.md'
                and path.name != 'README.md' and path not in pages):
            pages.append(path)
    expected = set(directory.rglob('*.md')) - {directory / 'README.md'}
    if set(pages) != expected:
        missing = sorted(str(p.relative_to(source)) for p in expected - set(pages))
        extra = sorted(str(p.relative_to(source)) for p in set(pages) - expected)
        raise ValueError(f'{section} README navigation differs from source pages: missing={missing}, extra={extra}')
    return pages


def expand(context, book):
    source = (Path(context['root']) / context['config']['book']['src']).resolve()
    roots = {f'{section}/README.md': section for section in SECTIONS}
    pending = set(roots)
    for item in book['items']:
        chapter = item.get('Chapter') if isinstance(item, dict) else None
        if chapter is None or chapter['path'] not in roots:
            continue
        pending.discard(chapter['path'])
        children = []
        for index, path in enumerate(ordered_pages(source, roots[chapter['path']]), start=1):
            content = path.read_text()
            heading = re.search(r'^# (.+)$', content, re.M)
            if not heading:
                raise ValueError(f'{path}: missing first-level heading')
            relative = path.relative_to(source).as_posix()
            number = chapter['number'] + [index] if chapter['number'] else None
            children.append({'Chapter': {
                'name': heading.group(1),
                'content': content,
                'number': number,
                'sub_items': [],
                'path': relative,
                'source_path': relative,
                'parent_names': chapter['parent_names'] + [chapter['name']],
            }})
        chapter['sub_items'] = children
    if pending:
        raise ValueError(f'SUMMARY missing section roots: {sorted(pending)}')
    return book


def main():
    if len(sys.argv) > 1:
        return 0 if sys.argv[1:] == ['supports', 'html'] else 1
    context, book = json.load(sys.stdin)
    json.dump(expand(context, book), sys.stdout)
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError) as error:
        print(f'docs navigation: {error}', file=sys.stderr)
        sys.exit(1)
