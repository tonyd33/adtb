import argparse
import json
import signal
from pathlib import Path
import re
import subprocess
import sys
import tempfile

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
    heading = {'spec': '## Parts\n', 'design': '## 7. Files\n'}.get(section)
    if heading:
        _, marker, index = index.partition(heading)
        if not marker:
            raise ValueError(f'{section} README missing navigation heading {heading.strip()!r}')
        index = index.split('\n## ', 1)[0]
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
        temporary = tempfile.NamedTemporaryFile(mode='w', dir=path.parent, delete=False)
        pending = Path(temporary.name)
        try:
            with temporary:
                temporary.write(text)
            pending.replace(path)
        finally:
            pending.unlink(missing_ok=True)


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
    # Keep the generated source visible to mdBook's watcher under an ignored target directory.
    write_changed(STAGE / '.gitignore', '')


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


def source_state():
    paths = [path for section, _ in SECTIONS for path in (ROOT / 'docs' / section).rglob('*.md')]
    paths += [TOOLS / 'index.md', TOOLS / 'adt-highlight.js']
    state = []
    for path in sorted(paths):
        metadata = path.stat()
        state.append((str(path), metadata.st_mtime_ns, metadata.st_size))
    return state


def serve(port):
    prepare()
    process = None

    def interrupted(signum, frame):
        raise KeyboardInterrupt

    handlers = {sig: signal.signal(sig, interrupted) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        process = subprocess.Popen(['mdbook', 'serve', str(STAGE), '--dest-dir', str(STAGE / 'served'),
                                    '--hostname', '127.0.0.1', '--port', str(port), '--watcher', 'poll'])
        previous, reported_error = None, None
        while True:
            try:
                current = source_state()
                if current != previous:
                    previous = current
                    prepare()
                    reported_error = None
            except (ValueError, OSError) as error:
                detail = str(error)
                if detail != reported_error:
                    print(f'docs: {detail}; waiting for a valid source edit', file=sys.stderr, flush=True)
                    reported_error = detail
            try:
                return process.wait(timeout=0.25)
            except subprocess.TimeoutExpired:
                pass
    except KeyboardInterrupt:
        return 0
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        for sig, handler in handlers.items():
            signal.signal(sig, handler)


def main():
    parser = argparse.ArgumentParser(description='Build, check or serve the local documentation site')
    parser.add_argument('command', choices=['build', 'check', 'serve'], nargs='?', default='build')
    parser.add_argument('--port', type=int, default=8081)
    args = parser.parse_args()
    if args.command == 'serve':
        return serve(args.port)
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
