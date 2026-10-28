from pathlib import Path
import re
import shutil
import signal
import socket
import subprocess
import tempfile
import time
import unittest
from urllib.error import HTTPError, URLError
from urllib.request import urlopen
from urllib.parse import urljoin


class ServeTest(unittest.TestCase):
    def test_native_server_rebuilds_navigation_and_assets(self):
        original = Path(__file__).resolve().parents[2]
        parent = original / 'target/docs-site-watch-tests'
        parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=parent) as temporary:
            root = Path(temporary)
            tools = root / 'tools/docs-site'
            tools.mkdir(parents=True)
            for name in ['navigation.py', 'adt-highlight.js']:
                shutil.copyfile(original / 'tools/docs-site' / name, tools / name)
            for name in ['book.toml', '.gitignore']:
                shutil.copyfile(original / name, root / name)
            for section, heading in [('manual', ''), ('spec', '## Parts\n'),
                                     ('design', '## 7. Files\n')]:
                directory = root / 'docs' / section
                directory.mkdir(parents=True)
                (directory / 'README.md').write_text(f'# {section}\n{heading}[A](a.md)\n[B](b.md)\n')
                (directory / 'a.md').write_text('# First\nInitial body.\n\n```adt\nfrom x in rows select x\n```\n')
                (directory / 'b.md').write_text('# Second\n')
            shutil.copyfile(original / 'docs/SUMMARY.md', root / 'docs/SUMMARY.md')
            (root / 'docs/index.md').write_text('# Documentation\nNative book fixture.\n')
            authored = [root / 'book.toml', root / 'docs/SUMMARY.md']
            before = [path.read_bytes() for path in authored]
            with socket.socket() as reservation:
                reservation.bind(('127.0.0.1', 0))
                port = reservation.getsockname()[1]
            address = f'http://127.0.0.1:{port}'
            log_path = root / 'serve.log'
            with log_path.open('w') as log:
                server = subprocess.Popen(['mdbook', 'serve', str(root), '--hostname',
                                           '127.0.0.1', '--port', str(port)], stdout=log, stderr=log)
                try:
                    self.wait_for(lambda: 'Initial body.' in self.fetch(address + '/manual/a.html'))
                    self.wait_for(lambda: 'Watching for changes' in log_path.read_text())
                    self.assertIn('language-adt', self.fetch(address + '/manual/a.html'))
                    for section in ['spec', 'design']:
                        self.assertIn('Initial body.', self.fetch(address + f'/{section}/a.html'))
                    manual = root / 'docs/manual'
                    (manual / 'new.md').write_text('# New chapter\nAdded while serving.\n')
                    (manual / 'README.md').write_text('# manual\n[B](b.md)\n[N](new.md)\n[A](a.md)\n')
                    self.wait_for(lambda: 'Added while serving.' in self.fetch(address + '/manual/new.html'))
                    self.wait_for(lambda: self.chapter_order(self.fetch(address + '/toc.html'),
                                                             'manual', ['b', 'new', 'a']))
                    html = self.fetch(address + '/index.html')
                    search = re.search(r'window.path_to_searchindex_js = "([^"]+)"', html).group(1)
                    self.assertIn('manual/new.html', self.fetch(urljoin(address + '/index.html', search)))
                    (manual / 'a.md').write_text('# Renamed first\nUpdated body.\n')
                    self.wait_for(lambda: 'Updated body.' in self.fetch(address + '/manual/a.html'))
                    self.wait_for(lambda: 'Renamed first' in self.fetch(address + '/toc.html'))
                    grammar = tools / 'adt-highlight.js'
                    grammar.write_text(grammar.read_text() + '\nwindow.docsWatchMarker = true;\n')
                    self.wait_for(lambda: 'docsWatchMarker' in self.script(address, 'adt-highlight'))
                    (root / 'docs/index.md').write_text('# Documentation\nUpdated home marker.\n')
                    self.wait_for(lambda: 'Updated home marker.' in self.fetch(address + '/index.html'))
                    (root / 'docs/spec/README.md').write_text('# Incomplete edit\n')
                    self.wait_for(lambda: 'missing navigation heading' in log_path.read_text())
                    self.assertIsNone(server.poll())
                    self.assertIn('Updated body.', self.fetch(address + '/manual/a.html'))
                    time.sleep(1.25)
                    self.assertEqual(1, log_path.read_text().count('missing navigation heading'))
                    (root / 'docs/spec/README.md').write_text('# spec\n## Parts\n[B](b.md)\n[A](a.md)\n')
                    self.wait_for(lambda: self.chapter_order(self.fetch(address + '/toc.html'),
                                                             'spec', ['b', 'a']))
                    (manual / 'new.md').unlink()
                    (manual / 'README.md').write_text('# manual\n[B](b.md)\n[A](a.md)\n')
                    self.wait_for(lambda: self.chapter_order(self.fetch(address + '/toc.html'),
                                                             'manual', ['b', 'a'], absent=['new']))
                    with self.assertRaises(HTTPError) as removed:
                        with urlopen(address + '/manual/new.html', timeout=0.5):
                            self.fail('removed chapter still has HTTP output')
                    with removed.exception as response:
                        self.assertEqual(404, response.code)
                    builds = log_path.read_text().count('Book building has started')
                    time.sleep(1.25)
                    self.assertEqual(builds, log_path.read_text().count('Book building has started'))
                    self.assertEqual(before, [path.read_bytes() for path in authored])
                    self.assertFalse((root / 'target/docs-site/source').exists())
                finally:
                    self.stop(server, signal.SIGTERM)
                    log.flush()
                    shutil.copyfile(log_path, parent / f'{root.name}-serve.log')
                self.wait_for(lambda: self.closed(port))
                with socket.socket() as occupied:
                    occupied.bind(('127.0.0.1', 0))
                    occupied.listen()
                    refused = subprocess.run(['mdbook', 'serve', str(root), '--hostname',
                                              '127.0.0.1', '--port', str(occupied.getsockname()[1])],
                                             stdout=log, stderr=log, timeout=10, check=False)
                    self.assertNotEqual(0, refused.returncode)
                restarted = subprocess.Popen(['mdbook', 'serve', str(root), '--hostname',
                                              '127.0.0.1', '--port', str(port)], stdout=log, stderr=log)
                try:
                    self.wait_for(lambda: 'Updated body.' in self.fetch(address + '/manual/a.html'))
                finally:
                    self.stop(restarted, signal.SIGINT)
                self.wait_for(lambda: self.closed(port))
                log.flush()
                shutil.copyfile(log_path, parent / f'{root.name}-complete.log')

    def stop(self, server, signum):
        server.send_signal(signum)
        try:
            server.wait(timeout=8)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait()
            self.fail('native mdBook did not stop')
        self.assertIn(server.returncode, [0, -signum])

    @classmethod
    def script(cls, address, name):
        html = cls.fetch(address + '/index.html')
        match = re.search(r'<script src="([^"]*' + name + r'[^"]*\.js)"', html)
        return cls.fetch(urljoin(address + '/index.html', match.group(1))) if match else ''

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
        self.fail('native docs condition did not become true within 10 seconds')
