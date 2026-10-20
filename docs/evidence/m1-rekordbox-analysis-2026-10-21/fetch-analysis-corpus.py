from pathlib import Path
import hashlib, json, subprocess, urllib.request
root = Path(r'C:\Users\Daniel\.codex\worktrees\m1-guition-bsp\research')
out = root/'anlz-public-corpus'
out.mkdir(exist_ok=True)
gh = r'C:\Program Files\GitHub CLI\gh.exe'
repo = 'Holzhaus/rekordcrate'
sha = '14d54eded7b0d2f6fe67c7871533dca35a73e5fa'
paths = ['data/anlz/sweep/ANLZ0000', 'data/complete_export/demo_tracks/PIONEER/USBANLZ/P016/0000875E/ANLZ0000', 'data/complete_export/demo_tracks/PIONEER/USBANLZ/P053/0001D21F/ANLZ0000']
manifest = {'repository': f'https://github.com/{repo}', 'commit': sha, 'license': 'MPL-2.0 repository; raw corpus retained outside product tree', 'pairs': []}
for index, path in enumerate(paths):
    pair = {'id': index, 'files': []}
    for extension in ['DAT', 'EXT']:
        url = f'https://raw.githubusercontent.com/{repo}/{sha}/{path}.{extension}'
        with urllib.request.urlopen(url, timeout=30) as response: data = response.read(8*1024*1024 + 1)
        assert len(data) <= 8*1024*1024
        name = f'{index}.{extension}'
        (out/name).write_bytes(data)
        pair['files'].append({'local': name, 'path': f'{path}.{extension}', 'url': url, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()})
    manifest['pairs'].append(pair)
(out/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
print(f'Pinned {len(paths)} DAT/EXT pairs at {sha}; raw files outside repository')
