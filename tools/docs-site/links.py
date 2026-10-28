from html.parser import HTMLParser
from urllib.parse import unquote, urlsplit


class Page(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.ids = set()
        self.links = []

    def handle_starttag(self, tag, attrs):
        data = dict(attrs)
        if 'id' in data:
            self.ids.add(data['id'])
        if tag == 'a' and 'name' in data:
            self.ids.add(data['name'])
        attribute = {'a': 'href', 'link': 'href', 'script': 'src', 'img': 'src'}.get(tag)
        if attribute and data.get(attribute):
            self.links.append(data[attribute])


def check(root):
    root = root.resolve()
    pages = {}
    for path in root.rglob('*.html'):
        page = Page()
        page.feed(path.read_text())
        pages[path] = page
    failures, links, fragments = [], 0, 0
    for path, page in pages.items():
        for link in page.links:
            url = urlsplit(link)
            if url.scheme or url.netloc:
                continue
            raw = unquote(url.path)
            target = (root / raw.lstrip('/') if raw.startswith('/') else path.parent / raw).resolve() if raw else path
            links += 1
            fragments += bool(url.fragment)
            if not target.is_relative_to(root):
                reason = 'path escapes site root'
            else:
                if target.is_dir():
                    target /= 'index.html'
                if not target.is_file():
                    reason = 'missing file'
                elif url.fragment and target in pages and unquote(url.fragment) not in pages[target].ids:
                    reason = 'missing anchor'
                else:
                    continue
            failures.append(f'{path.relative_to(root)}: {link}: {reason}')
    print(f'docs: {len(pages)} HTML pages, {links} internal links, {fragments} fragments, {len(failures)} failures')
    for failure in failures:
        print(failure)
    return failures
