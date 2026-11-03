from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from pack_placement import relocation_args, functions, verify, verify_relocation

SYMBOLS = (
    '_ZN10ruzina_uac6packet4pack17h123abcE',
    '_ZN10ruzina_uac6packet4pack28_$u7b$$u7b$closure$u7d$$u7d$17h123abcE',
    '_ZN10ruzina_uac6packet5write17h123abcE',
)


def table(section=None):
    return 'crate.hash-cgu.0.rcgu.o:     file format elf32-littleriscv\n' + '\n'.join(
        f'00000000 g F {section or ".text." + symbol} 00000100 {symbol}' for symbol in SYMBOLS)


class PackPlacementTests(unittest.TestCase):
    def test_instruction_and_relocation_drift_rejected(self):
        moves=relocation_args(table())
        source='instruction .text.'+SYMBOLS[0]+' relocation'
        target=source.replace('.text.'+SYMBOLS[0],'.iram1.ruzina_pack_0')
        verify_relocation(source,target,moves)
        with self.assertRaises(ValueError):
            verify_relocation(source,target.replace('instruction','changed'),moves)
    def test_exact_symbol_fragment_excludes_other_functions(self):
        text = table() + '\n00000000 g F .text.other 00000100 other'
        result = relocation_args(text)
        self.assertEqual(result.count('--rename-section'), 3)
        self.assertTrue(all('other' not in item and '*' not in item for item in result))
        self.assertIn('.text.' + SYMBOLS[0] + '=.iram1.ruzina_pack_0', result)

    def test_undefined_references_do_not_count_as_definitions(self):
        self.assertEqual(len(functions(table() + '\n00000000 *UND* 00000000 ' + SYMBOLS[0])), 3)

    def test_missing_duplicate_oversized_and_merged_sections_rejected(self):
        for bad in (table().replace(SYMBOLS[2], 'unknown'), table() + '\n' + table(),
                    table().replace('00000100', '00010000'), table('.text')):
            with self.assertRaises(ValueError):
                relocation_args(bad)

    def test_linked_control_and_variant_are_verified(self):
        verify(table('.flash.text'), False)
        verify(table('.iram0.text'), True)
        with self.assertRaises(ValueError):
            verify(table('.flash.text'), True)
        with self.assertRaises(ValueError):
            verify(table('.iram0.text'), False)


if __name__ == '__main__':
    unittest.main()
