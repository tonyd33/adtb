from pathlib import Path
import subprocess
import sys
import tomllib

from links import check as check_links
from syntax import check as check_syntax


ROOT = Path(__file__).resolve().parents[2]


def main():
    if len(sys.argv) != 1:
        raise ValueError('usage: work/tools/docs-site check')
    with (ROOT / 'book.toml').open('rb') as config:
        book = tomllib.load(config)
    source = ROOT / book['book']['src']
    output = ROOT / book['build']['build-dir']
    subprocess.run(['mdbook', 'build', str(ROOT)], check=True)
    subprocess.run([sys.executable, '-B', '-m', 'unittest', 'discover',
                    '-s', str(ROOT / 'tools/docs-site'), '-p', 'test_*.py'], check=True)
    if check_links(output):
        return 1
    check_syntax(ROOT, source, output)
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f'docs check: {error}', file=sys.stderr)
        sys.exit(1)
