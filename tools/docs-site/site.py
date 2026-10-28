import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys

from links import check

ROOT = Path(__file__).resolve().parents[2]
TOOLS = ROOT / 'tools/docs-site'
STAGE = ROOT / 'target/docs-site'
SOURCE = STAGE / 'source'
OUTPUT = STAGE / 'html'
SECTIONS = [('manual', 'Manual'), ('spec', 'Specification'), ('design', 'Design')]


def ordered_pages(section):
    directory = ROOT / 'docs' / section
    index = (directory / 'README.md').read_text()
    if section == 'spec':
        index = index.split('## Parts\n', 1)[1].split('\n## ', 1)[0]
    elif section == 'design':
        index = index.split('## 7. Files\n', 1)[1].split('\n## ', 1)[0]
    pages = []
    for target in re.findall(r'\]\(([^)#]+)(?:#[^)]*)?\)', index):
        path = (directory / target).resolve()
        if path.is_relative_to(directory) and path.suffix == '.md' and path.name != 'README.md' and path not in pages:
            pages.append(path)
    expected = set(directory.rglob('*.md')) - {directory / 'README.md'}
    if set(pages) != expected:
        missing = sorted(str(p.relative_to(ROOT)) for p in expected - set(pages))
        extra = sorted(str(p) for p in set(pages) - expected)
        raise ValueError(f'{section} README navigation differs from source pages: missing={missing}, extra={extra}')
    return pages


def title(path):
    match = re.search(r'^# (.+)$', path.read_text(), re.M)
    if not match:
        raise ValueError(f'{path}: missing first-level heading')
    return match.group(1).replace('[', '').replace(']', '')


def write_changed(path, text):
    if not path.exists() or path.read_text() != text:
        path.write_text(text)


def prepare():
    SOURCE.mkdir(parents=True, exist_ok=True)
    summary = ['# Summary', '', '- [Documentation](index.md)']
    for section, label in SECTIONS:
        link = SOURCE / section
        destination = ROOT / 'docs' / section
        if link.is_symlink():
            if link.resolve() != destination:
                raise ValueError(f'{link}: unexpected symlink target')
        elif link.exists():
            raise ValueError(f'{link}: expected a source symlink')
        else:
            link.symlink_to(destination, target_is_directory=True)
        summary.extend(['', f'- [{label}]({section}/README.md)'])
        for path in ordered_pages(section):
            relative = path.relative_to(destination)
            summary.append(f'  - [{title(path)}]({section}/{relative.as_posix()})')
    write_changed(SOURCE / 'SUMMARY.md', '\n'.join(summary) + '\n')
    write_changed(SOURCE / 'index.md', (TOOLS / 'index.md').read_text())
    write_changed(SOURCE / 'adt-highlight.js', (TOOLS / 'adt-highlight.js').read_text())
    config = f'''[book]
title = "adtb documentation"
language = "en"
src = "source"
[build]
build-dir = "html"
create-missing = false
use-default-preprocessors = false
extra-watch-dirs = [{json.dumps(str(ROOT / 'docs'))}]
[preprocessor.links]
[output.html]
no-section-label = true
additional-js = ["source/adt-highlight.js"]
[output.html.playground]
runnable = false
'''
    write_changed(STAGE / 'book.toml', config)


def fences():
    result = []
    for section, _ in SECTIONS:
        for path in sorted((ROOT / 'docs' / section).rglob('*.md')):
            lines = path.read_text().splitlines(keepends=True)
            index = 0
            while index < len(lines):
                start = re.match(r'^(\s*)(`{3,}|~{3,})adt(?:\s|$)', lines[index])
                if not start:
                    index += 1
                    continue
                marker = start.group(2)
                source, first = [], index + 1
                index += 1
                while index < len(lines) and not re.match(r'^\s*' + re.escape(marker[0]) + '{' + str(len(marker)) + r',}\s*$', lines[index]):
                    line = lines[index]
                    source.append(line[len(start.group(1)):] if line.startswith(start.group(1)) else line)
                    index += 1
                if index == len(lines):
                    raise ValueError(f'{path}:{first}: unclosed adt fence')
                result.append({'name': f'{path.relative_to(ROOT)}:{first}', 'source': ''.join(source), 'tokens': []})
                index += 1
    return result


def syntax_check():
    lexer = (ROOT / 'crates/adtb-lang/src/lex.rs').read_text()
    keywords = re.findall(r'^\s*"([a-z]+)" => [A-Z]', lexer, re.M)
    grammar = (TOOLS / 'adt-highlight.js').read_text()
    actual = re.search(r"keyword: '([^']+)'", grammar).group(1).split()
    if sorted(actual) != sorted(keywords):
        raise ValueError('adt grammar keywords differ from lexer Kw::from_word')
    corpus = fences()
    for path in sorted((ROOT / 'tools/syntax/corpus').glob('*.adt')):
        corpus.append({'name': str(path.relative_to(ROOT)), 'source': path.read_text(),
                       'tokens': json.loads(path.with_suffix('.json').read_text())})
    corpus.append({'name': 'every lexer keyword', 'source': ' '.join(keywords),
                   'tokens': [['keyword', word] for word in keywords]})
    corpus.append({'name': 'keyword boundaries', 'source': 'from_here falsehood MyType',
                   'tokens': [['type', 'MyType']], 'absent': ['keyword']})
    (STAGE / 'syntax-corpus.json').write_text(json.dumps(corpus))
    highlights = list(OUTPUT.glob('highlight-*.js'))
    if len(highlights) != 1:
        raise ValueError(f'expected one mdBook Highlight.js runtime, found {highlights}')
    runner = STAGE / 'syntax-check.js'
    runner.write_text('var window = {};\n' + highlights[0].read_text() + '\n' + grammar +
                      '\nvar corpus = ' + json.dumps(corpus) + ';\n' +
                      (TOOLS / 'syntax-check.js').read_text())
    subprocess.run(['qjs', str(runner)], check=True)


def build():
    prepare()
    subprocess.run(['mdbook', 'build', str(STAGE)], check=True)
    expected = [ROOT / 'docs' / name / 'README.md' for name, _ in SECTIONS]
    expected += [p for name, _ in SECTIONS for p in ordered_pages(name)]
    for page in expected:
        output = OUTPUT / page.relative_to(ROOT / 'docs').with_suffix('.html')
        if not output.is_file():
            raise ValueError(f'{page}: missing output {output}')


def main():
    parser = argparse.ArgumentParser(description='Build, check or serve the local documentation site')
    parser.add_argument('command', choices=['build', 'check', 'serve'], nargs='?', default='build')
    parser.add_argument('--port', type=int, default=8081)
    args = parser.parse_args()
    if args.command == 'serve':
        prepare()
        os.execvp('mdbook', ['mdbook', 'serve', str(STAGE), '--dest-dir', str(STAGE / 'served'),
                              '--hostname', '127.0.0.1', '--port', str(args.port)])
    build()
    if args.command == 'check':
        subprocess.run([sys.executable, '-m', 'unittest', 'discover', '-s', str(TOOLS), '-p', 'test_*.py'], check=True)
        if check(OUTPUT):
            return 1
        syntax_check()
    print(f'docs: built {OUTPUT}/index.html')
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f'docs: {error}', file=sys.stderr)
        sys.exit(1)
