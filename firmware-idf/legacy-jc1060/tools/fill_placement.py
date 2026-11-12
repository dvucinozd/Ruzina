"""Moves the USB audio fill's code to internal RAM, by exact function sections.

Next to the screen, a fill stalls on flash and PSRAM cache misses. Code that runs per
frame or per block therefore moves to IRAM. Only whole function sections move, and
instructions stay identical, as `verify_relocation` checks.
"""
import re

# Per frame or per block on the fill path. Load, seek and control paths stay in flash.
FILL_PATH = re.compile(
    r'(?:ruzina_legacy_uac_fill'
    r'|_ZN16ruzina_audio_dsp'
    r'|_ZN20ruzina_audio_runtime6render'
    r'|_ZN20ruzina_audio_runtime2fx'
    r'|_ZN20ruzina_audio_scratch'
    r'|_ZN28ruzina_legacy_idf_core_probe12deck_keylock'
    r'|_ZN12ruzina_mixer'
    r'|_ZN10ruzina_uac6packet'
    r'|_ZN28ruzina_legacy_idf_core_probe8deck_mix'
    r'|_ZN28ruzina_legacy_idf_core_probe9deck_play6Reader'
    r'|_ZN28ruzina_legacy_idf_core_probe16controller_audio)'
)
MAX_BYTES = 256 * 1024


def function_row(line):
    """A function's symbol, section and size from an `objdump -t` row, which may carry
    a visibility word such as `.hidden` before the symbol."""
    words = line.split()
    if 'F' not in words or words.index('F') + 2 >= len(words):
        return None
    at = words.index('F')
    try:
        return words[-1], words[at + 1], int(words[at + 2], 16)
    except ValueError:
        return None


def functions(table):
    """Defined functions on the fill path that have their own section."""
    found = []
    obj = None
    for line in table.splitlines():
        if ':     file format ' in line:
            obj = line.split(':', 1)[0]
        parsed = function_row(line)
        if not parsed or '*UND*' in line:
            continue
        symbol, section, size = parsed
        if FILL_PATH.match(symbol) and section == '.text.' + symbol:
            found.append(dict(object=obj, symbol=symbol, section=section, bytes=size))
    if not found:
        raise ValueError('Fill path functions not found: section layout changed')
    total = sum(item['bytes'] for item in found)
    if total > MAX_BYTES:
        raise ValueError(f'Fill path code of {total} bytes exceeds the IRAM budget')
    return found


def relocation_args(table):
    moves = []
    for i, item in enumerate(functions(table)):
        moves += ['--rename-section', item['section'] + '=.iram1.ruzina_fill_' + str(i)]
    return moves


def verify_relocation(original, placed, moves):
    """Only the moved section names may change in code and relocation dumps."""
    old = [spec.split('=')[0] for spec in moves[1::2]]
    restored = re.sub(r'\.iram1\.ruzina_fill_(\d+)', lambda m: old[int(m.group(1))], placed)
    if original != restored:
        raise ValueError('Archive instruction/relocation drift')


def verify(table):
    """Every fill path function that moved must have linked into IRAM."""
    placed = {}
    for line in table.splitlines():
        parsed = function_row(line)
        if parsed and FILL_PATH.match(parsed[0]):
            placed[parsed[0]] = parsed[1]
    moved = {name: section for name, section in placed.items() if section == '.iram0.text'}
    if not moved or 'ruzina_legacy_uac_fill' not in moved:
        raise ValueError('Fill path did not link into IRAM')
    return dict(iram=sorted(moved), flash=sorted(set(placed) - set(moved)))
