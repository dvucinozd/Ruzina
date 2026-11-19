from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fill_placement import relocation_args, functions, verify_relocation

FILL = '_ZN16ruzina_audio_dsp7EqState13process_frame17h123abcE'
OTHER = '_ZN28ruzina_legacy_idf_core_probe8deck_mix3Mix6render17h456defE'


def row(symbol, size=0x100):
    return f'00000000 l F .text.{symbol} {size:08x} {symbol}'


def table(*objects):
    return '\n'.join(f'{name}:     file format elf32-littleriscv\n' + '\n'.join(rows)
                     for name, rows in objects)


class FillPlacementTests(unittest.TestCase):
    def test_an_inline_function_in_two_objects_moves_once(self):
        text = table(('a-cgu.0.rcgu.o', [row(FILL), row(OTHER)]), ('b-cgu.0.rcgu.o', [row(FILL)]))
        self.assertEqual(len(functions(text)), 3)
        self.assertEqual(relocation_args(text), [
            '--rename-section', f'.text.{FILL}=.iram1.ruzina_fill_0',
            '--rename-section', f'.text.{OTHER}=.iram1.ruzina_fill_1'])

    def test_both_copies_count_toward_the_budget(self):
        text = table(('a.o', [row(FILL, 0x21000)]), ('b.o', [row(FILL, 0x21000)]))
        with self.assertRaises(ValueError):
            functions(text)

    def test_relocation_drift_rejected(self):
        moves = relocation_args(table(('a.o', [row(FILL)])))
        verify_relocation(f'call .text.{FILL}', 'call .iram1.ruzina_fill_0', moves)
        with self.assertRaises(ValueError):
            verify_relocation(f'call .text.{FILL}', 'call .iram1.ruzina_fill_0 changed', moves)


if __name__ == '__main__':
    unittest.main()
