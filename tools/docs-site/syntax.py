import json
import re
import subprocess


def fences(source):
    result = []
    for path in sorted(source.rglob('*.md')):
        lines = path.read_text().splitlines(keepends=True)
        index = 0
        while index < len(lines):
            start = re.match(r'^(\s*)(`{3,}|~{3,})adt(?:\s|$)', lines[index])
            if not start:
                index += 1
                continue
            marker = start.group(2)
            content, first = [], index + 1
            index += 1
            while index < len(lines) and not re.match(r'^\s*' + re.escape(marker[0]) + '{' + str(len(marker)) + r',}\s*$', lines[index]):
                line = lines[index]
                content.append(line[len(start.group(1)):] if line.startswith(start.group(1)) else line)
                index += 1
            if index == len(lines):
                raise ValueError(f'{path}:{first}: unclosed adt fence')
            result.append({'name': f'{path.relative_to(source)}:{first}', 'source': ''.join(content), 'tokens': []})
            index += 1
    return result


def check(root, source, output):
    tools = root / 'tools/docs-site'
    artifacts = root / 'target/docs-site-check'
    artifacts.mkdir(parents=True, exist_ok=True)
    lexer = (root / 'crates/adtb-lang/src/lex.rs').read_text()
    keywords = re.findall(r'^\s*"([a-z]+)" => [A-Z]', lexer, re.M)
    grammar = (tools / 'adt-highlight.js').read_text()
    actual = re.search(r"keyword: '([^']+)'", grammar).group(1).split()
    if sorted(actual) != sorted(keywords):
        raise ValueError('adt grammar keywords differ from lexer Kw::from_word')
    corpus = fences(source)
    for path in sorted((root / 'tools/syntax/corpus').glob('*.adt')):
        corpus.append({'name': str(path.relative_to(root)), 'source': path.read_text(),
                       'tokens': json.loads(path.with_suffix('.json').read_text())})
    corpus.append({'name': 'every lexer keyword', 'source': ' '.join(keywords),
                   'tokens': [['keyword', word] for word in keywords]})
    corpus.append({'name': 'keyword boundaries', 'source': 'from_here falsehood MyType',
                   'tokens': [['type', 'MyType']], 'absent': ['keyword']})
    (artifacts / 'syntax-corpus.json').write_text(json.dumps(corpus))
    highlights = list(output.glob('highlight-*.js'))
    if len(highlights) != 1:
        raise ValueError(f'expected one mdBook Highlight.js runtime, found {highlights}')
    runner = artifacts / 'syntax-check.js'
    runner.write_text('var window = {};\n' + highlights[0].read_text() + '\n' + grammar +
                      '\nvar corpus = ' + json.dumps(corpus) + ';\n' +
                      (tools / 'syntax-check.js').read_text())
    subprocess.run(['qjs', str(runner)], check=True)
