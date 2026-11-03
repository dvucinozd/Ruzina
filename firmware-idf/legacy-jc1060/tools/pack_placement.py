"""Exact-symbol placement for one OFF-by-default packetizer experiment."""
import re

PREFIX = '_ZN10ruzina_uac6packet'
PATTERNS = (
    re.compile(PREFIX + r'4pack17h[0-9a-f]+E'),
    re.compile(re.escape(PREFIX + '4pack28_$u7b$$u7b$closure$u7d$$u7d$') + r'17h[0-9a-f]+E'),
    re.compile(PREFIX + r'5write17h[0-9a-f]+E'),
)


def functions(table):
    """Require exactly the audited three function definitions, never UND refs."""
    found = [[] for _ in PATTERNS]
    obj = None
    for line in table.splitlines():
        if ':     file format ' in line:
            obj = line.split(':', 1)[0]
        words = line.split()
        if len(words) < 6 or 'F' not in words or '*UND*' in words:
            continue
        symbol = words[-1]
        for i, pattern in enumerate(PATTERNS):
            if pattern.fullmatch(symbol):
                found[i].append(dict(object=obj, symbol=symbol, section=words[-3], bytes=int(words[-2], 16)))
    if any(len(items) != 1 for items in found):
        raise ValueError('Packetizer layout drift: require exactly pack/closure/write')
    result = [items[0] for items in found]
    if not 0 < sum(item['bytes'] for item in result) <= 2048:
        raise ValueError('Packetizer code size exceeds the bounded experiment')
    return result


def relocation_args(table):
    selected = functions(table)
    entries = []
    for i, item in enumerate(selected):
        obj = item['object']
        if not obj or not re.fullmatch(r'[\w.\-]+\.o', obj):
            raise ValueError('Missing or unsupported archive member')
        if item['section'] != '.text.' + item['symbol']:
            raise ValueError('Packetizer requires separate exact function sections')
        entries += ['--rename-section', item['section'] + '=.iram1.ruzina_pack_' + str(i)]
    return entries


def verify(table, iram):
    selected = functions(table)
    expected = '.iram0.text' if iram else '.flash.text'
    if any(item['section'] != expected for item in selected):
        raise ValueError('Packetizer linked placement mismatch: ' + expected)
    return selected


def verify_relocation(original, placed, moves):
    """Only the three exact section names may change in code/relocation dumps."""
    for spec in moves[1::2]:
        old, new = spec.split('=')
        placed = placed.replace(new, old)
    if original != placed:
        raise ValueError('Archive instruction/relocation drift')
