import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest
from urllib.error import HTTPError, URLError
from urllib.request import urlopen


class ServeTest(unittest.TestCase):
    def test_live_navigation_assets_and_shutdown(self):
        original = Path(__file__).resolve().parents[2]
        parent = original / 'target/docs-site-watch-tests'
        parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=parent) as temporary:
            root = Path(temporary)
            tools = root / 'tools/docs-site'
            tools.mkdir(parents=True)
            for name in ['site.py', 'links.py', 'index.md', 'adt-highlight.js']:
                shutil.copyfile(original / 'tools/docs-site' / name, tools / name)
            for section, heading in [('manual', ''), ('spec', '## Parts\n'),
                                     ('design', '## 7. Files\n')]:
                directory = root / 'docs' / section
                directory.mkdir(parents=True)
                (directory / 'README.md').write_text(f'# {section}\n{heading}[A](a.md)\n[B](b.md)\n')
                (directory / 'a.md').write_text('# First\nInitial body.\n')
                (directory / 'b.md').write_text('# Second\n')
            with socket.socket() as reservation:
                reservation.bind(('127.0.0.1', 0))
                port = reservation.getsockname()[1]
            address = f'http://127.0.0.1:{port}'
            log_path = root / 'serve.log'
            stage = root / 'target/docs-site/source'
            summary = stage / 'SUMMARY.md'
            with log_path.open('w') as log:
                server = subprocess.Popen([sys.executable, '-B', str(tools / 'site.py'),
                                           'serve', '--port', str(port)], stdout=log, stderr=log,
                                          env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'})
                try:
                    self.wait_for(lambda: 'Initial body.' in self.fetch(address + '/manual/a.html'))
                    self.wait_for(lambda: 'Watching for changes' in log_path.read_text())
                    self.assertIn('manual/a.md', summary.read_text())
                    manual = root / 'docs/manual'
                    (manual / 'new.md').write_text('# New chapter\nAdded while serving.\n')
                    (manual / 'README.md').write_text('# manual\n[B](b.md)\n[N](new.md)\n[A](a.md)\n')
                    self.wait_for(lambda: 'Added while serving.' in self.fetch(address + '/manual/new.html'))
                    self.wait_for(lambda: self.chapter_order(self.fetch(address + '/toc.html'), 'manual', ['b', 'new', 'a']))
                    self.assertLess(summary.read_text().index('manual/b.md'),
                                    summary.read_text().index('manual/new.md'))
                    self.assertLess(summary.read_text().index('manual/new.md'),
                                    summary.read_text().index('manual/a.md'))
                    (manual / 'a.md').write_text('# Renamed first\nUpdated body.\n')
                    self.wait_for(lambda: 'Updated body.' in self.fetch(address + '/manual/a.html'))
                    self.wait_for(lambda: 'Renamed first' in summary.read_text())
                    grammar = tools / 'adt-highlight.js'
                    grammar.write_text(grammar.read_text() + '\nwindow.docsWatchMarker = true;\n')
                    self.wait_for(lambda: 'docsWatchMarker' in self.fetch(address + '/adt-highlight.js'))
                    self.assertIn('docsWatchMarker', (stage / 'adt-highlight.js').read_text())
                    index = tools / 'index.md'
                    index.write_text(index.read_text() + '\nUpdated home marker.\n')
                    self.wait_for(lambda: 'Updated home marker.' in self.fetch(address + '/index.html'))
                    valid_summary = summary.read_text()
                    (root / 'docs/spec/README.md').write_text('# Incomplete edit\n')
                    self.wait_for(lambda: 'missing navigation heading' in log_path.read_text())
                    self.assertIsNone(server.poll())
                    self.assertEqual(valid_summary, summary.read_text())
                    time.sleep(0.6)
                    self.assertEqual(1, log_path.read_text().count('missing navigation heading'))
                    (root / 'docs/spec/README.md').write_text('# spec\n## Parts\n[B](b.md)\n[A](a.md)\n')
                    self.wait_for(lambda: summary.read_text().index('spec/b.md') <
                                  summary.read_text().index('spec/a.md'))
                    self.wait_for(lambda: self.chapter_order(self.fetch(address + '/toc.html'),
                                                             'spec', ['b', 'a']))
                    (manual / 'new.md').unlink()
                    (manual / 'README.md').write_text('# manual\n[B](b.md)\n[A](a.md)\n')
                    self.wait_for(lambda: 'manual/new.md' not in summary.read_text())
                    self.wait_for(lambda: self.chapter_order(self.fetch(address + '/toc.html'),
                                                             'manual', ['b', 'a'], absent=['new']))
                    watched = [summary, stage / 'adt-highlight.js', stage / 'index.md',
                               root / 'target/docs-site/book.toml']
                    time.sleep(1)
                    stable = [path.stat().st_mtime_ns for path in watched]
                    builds = log_path.read_text().count('Book building has started')
                    time.sleep(1.25)
                    self.assertEqual(stable, [path.stat().st_mtime_ns for path in watched])
                    self.assertEqual(builds, log_path.read_text().count('Book building has started'))
                    self.assertIsNone(server.poll())
                finally:
                    server.terminate()
                    try:
                        server.wait(timeout=8)
                    except subprocess.TimeoutExpired:
                        server.kill()
                        server.wait()
                        self.fail('docs supervisor did not stop')
                    log.flush()
                    shutil.copyfile(log_path, parent / f'{root.name}-serve.log')
                    if summary.exists():
                        shutil.copyfile(summary, parent / 'last-SUMMARY.md')
                self.assertEqual(0, server.returncode, log_path.read_text())
                self.wait_for(lambda: self.closed(port))
                with socket.socket() as occupied:
                    occupied.bind(('127.0.0.1', 0))
                    occupied.listen()
                    refused = subprocess.run([sys.executable, '-B', str(tools / 'site.py'),
                                              'serve', '--port', str(occupied.getsockname()[1])],
                                             stdout=log, stderr=log, timeout=10, check=False)
                    self.assertNotEqual(0, refused.returncode)
                restarted = subprocess.Popen([sys.executable, '-B', str(tools / 'site.py'),
                                              'serve', '--port', str(port)], stdout=log, stderr=log)
                try:
                    self.wait_for(lambda: 'Updated body.' in self.fetch(address + '/manual/a.html'))
                finally:
                    restarted.send_signal(signal.SIGINT)
                    try:
                        restarted.wait(timeout=8)
                    except subprocess.TimeoutExpired:
                        restarted.kill()
                        restarted.wait()
                        self.fail('docs supervisor did not stop on SIGINT')
                self.assertEqual(0, restarted.returncode, log_path.read_text())
                self.wait_for(lambda: self.closed(port))
                log.flush()
                shutil.copyfile(log_path, parent / f'{root.name}-complete.log')

    @staticmethod
    def chapter_order(html, section, names, absent=()):
        positions = [html.find(f'{section}/{name}.html') for name in names]
        return (all(position >= 0 for position in positions) and positions == sorted(positions) and
                all(f'{section}/{name}.html' not in html for name in absent))

    @staticmethod
    def fetch(address):
        try:
            with urlopen(address, timeout=0.5) as response:
                return response.read().decode()
        except HTTPError as error:
            error.close()
            return ''
        except (URLError, TimeoutError, ConnectionError):
            return ''

    @staticmethod
    def closed(port):
        with socket.socket() as connection:
            connection.settimeout(0.5)
            return connection.connect_ex(('127.0.0.1', port)) != 0

    def wait_for(self, condition):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if condition():
                return
            time.sleep(0.1)
        self.fail('live docs condition did not become true within 10 seconds')
