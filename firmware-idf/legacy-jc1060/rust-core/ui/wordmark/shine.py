"""The wordmark's shine: the six letters side by side (720 x 112, as the boot screen and the
screensaver lay them out) and brightened, for the band that slides across the word.
Run it after make.sh, with Pillow: python shine.py"""
from pathlib import Path

from PIL import Image, ImageEnhance

here = Path(__file__).resolve().parent
letters = [Image.open(here / f'{letter}.png').convert('RGBA') for letter in 'RUZINA']
word = Image.new('RGBA', (sum(letter.width for letter in letters), max(letter.height for letter in letters)))
x = 0
for letter in letters:
    word.alpha_composite(letter, (x, 0))
    x += letter.width
assert word.size == (720, 112), word.size
ImageEnhance.Brightness(word).enhance(1.9).save(here / 'word-shine.png', optimize=True)
print('word-shine.png', word.size)
