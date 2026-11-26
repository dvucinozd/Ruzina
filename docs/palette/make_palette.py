"""The board screen's palette as one image: every colour the deck screen uses, grouped,
with the elements it is on. Colours come from `rust-core/ui/overview.slint` (Slint) and the
RGB565 constants in `rust-core/src/slint_ui.rs` (what Rust draws itself: waveforms, lines,
cues, meters, the phase strip). The panel shows RGB565, so each swatch is shown as written
and as the panel shows it.

    python docs/palette/make_palette.py   ->  docs/palette/ruzina-screen-palette.png
"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

FONTS = Path('C:/Windows/Fonts')
TITLE = ImageFont.truetype(str(FONTS / 'segoeuib.ttf'), 34)
HEAD = ImageFont.truetype(str(FONTS / 'segoeuib.ttf'), 22)
TEXT = ImageFont.truetype(str(FONTS / 'segoeui.ttf'), 17)
MONO = ImageFont.truetype(str(FONTS / 'consolab.ttf'), 18)
SMALL = ImageFont.truetype(str(FONTS / 'consola.ttf'), 14)

BACK = '#100c08'
LINE = '#38251e'
INK = '#f5f0ed'
MUTED = '#a3958e'

# (group, [(hex, name, where)])
GROUPS = [
    ('Pozadine i površine', [
        ('#100c08', 'Glavna pozadina', 'pozadina ekrana i waveform lanea, Library, Settings, FX, screensaver i UPDATING; tekst na svijetlim gumbima (D1/D2, LOAD, PLAY, CUE)'),
        ('#1e1511', 'Površina', 'kartice decka, rail, redovi Library (parni u Settings), isključeni gumbi, pozadina phase stripa'),
        ('#181210', 'Površina 2', 'neparni redovi na Settings'),
        ('#160f0c', 'Površina FX', 'pozadina FX panela, kanal na koji efekt ne ide'),
        ('#2a1b14', 'Aktivni rail / meter ugašen', 'pozadina aktivne ikone na railu; ugašeni segment metera'),
        ('#0b2614', 'Tamno zelena', 'MT uključen (pozadina), odabrani red u Library, BACK pritisnut'),
        ('#2a2300', 'Tamno žuta', 'QUANTIZE uključen (pozadina)'),
        ('#301c00', 'Sjena loopa', 'raspon loopa iza waveforma'),
    ]),
    ('Linije i rubovi', [
        ('#38251e', 'Rub', 'rubovi kartica, linija raila, razdjelnik Hot Cues, rub izbora; beat linije u waveformu; odsvirani dio overviewa; sredina phase stripa'),
        ('#574b45', 'Prigušeni rub', 'isključeni MT/Q i FX rubovi, BACK isključen, faza "—"; bar linije u waveformu'),
    ]),
    ('Tekst', [
        ('#f5f0ed', 'Glavni tekst', 'naslovi, proteklo vrijeme, BPM i TIME u Library, vrijednosti postavki; playhead, mini playhead, memory cue, bijeli vrhovi waveforma'),
        ('#a3958e', 'Sekundarni tekst', 'preostalo vrijeme, oznake (BPM, EFFECT, TARGET…), neaktivne ikone raila, status, "touch me…", podnaslov UPDATING; key koji ne pristaje'),
        ('#f0e6e0', 'Naslov UPDATING', 'UPDATING FIRMWARE'),
    ]),
    ('Decki i naglasci', [
        ('#e64c0c', 'Deck 1 / Rust narančasta', 'D1, LOAD D1, pruga D1 na kartici, aktivna ikona raila, BPM broj, naslovi u Settings, decki u Library redu, playlist, BPM filter rub; niski u waveformu; boja D1 u phase stripu'),
        ('#00e5ff', 'Deck 2 / cijan', 'D2, LOAD D2, pruga D2 na kartici; visoki u waveformu; boja D2 u phase stripu; Hot Cue B'),
        ('#b7410e', 'Hrđa', 'PAUSE, odabrani izbor u Settings, zaglavlje Library, BPM filter uključen, FX DEPTH traka'),
        ('#ff7700', 'Narančasta', 'tonalitet (key) u decku, LOOP u memory cueovima, oznaka foldera, key nepoznat; rubovi loopa u waveformu'),
    ]),
    ('Stanja', [
        ('#00ff66', 'Zelena', 'PLAY, MT, odabrani red i rub u Library, BACK, FX ON, key pristaje; Hot Cue A'),
        ('#ffd600', 'Žuta', 'CUE, pitch, QUANTIZE, sortirani stupac, faza drugog decka, key +2; Hot Cue D'),
        ('#ff2e63', 'Crvena brisanja', 'DELETE na Hot Cues i rub pločice koja se briše'),
        ('#0088ff', 'Plava', 'key +7; srednji u waveformu'),
    ]),
    ('Waveform (frekvencije)', [
        ('#e64c0c', 'Niski', 'boja 0–2 bjeline iz rekordboxa'),
        ('#0088ff', 'Srednji', 'boja 3–4'),
        ('#00e5ff', 'Visoki', 'boja 5–7'),
        ('#39ff14', 'Bar trokut', 'zeleni trokut na svakoj bar liniji'),
    ]),
    ('Meteri', [
        ('#39ff14', 'Do 0 dB', 'acid zelena'),
        ('#e6193c', 'Iznad 0 dB', 'koraljno crvena (u kodu METER_AMBER)'),
        ('#ff0033', 'Vrh', 'laser crvena; i Hot Cue C'),
    ]),
    ('Hot Cue A–H', [
        ('#00ff66', 'A', 'linija i zastavica u waveformu, pločica na Hot Cues'),
        ('#00e5ff', 'B', ''),
        ('#ff0033', 'C', ''),
        ('#ffd600', 'D', ''),
        ('#a259ff', 'E', ''),
        ('#ff2ec4', 'F', ''),
        ('#2f6bff', 'G', ''),
        ('#b6ff00', 'H', ''),
    ]),
    ('FX efekti', [
        ('#eb870f', 'ECHO', 'boja FX panela kad je efekt uključen'),
        ('#b44ae0', 'FLANGER', ''),
        ('#27c7c7', 'DELAY', ''),
        ('#2d85cd', 'Ostali', ''),
    ]),
]

# Colours close enough to confuse, for the screen-by-screen pass.
NEAR = [
    ('#ff0033', '#ff2e63', '#e6193c', 'tri crvene: vrh metera i Hot Cue C, brisanje, iznad 0 dB'),
    ('#00ff66', '#39ff14', '#b6ff00', 'tri zelene: stanja i Hot Cue A, meter i bar trokut, Hot Cue H'),
    ('#e64c0c', '#ff7700', '#eb870f', 'tri narančaste: Deck 1, key i loop, ECHO'),
    ('#f5f0ed', '#f0e6e0', None, 'dvije bijele: glavni tekst, naslov UPDATING'),
]


def rgb(hex_text):
    value = int(hex_text[1:], 16)
    return value >> 16 & 255, value >> 8 & 255, value & 255


def panel(hex_text):
    """The colour as the RGB565 panel shows it."""
    r, g, b = rgb(hex_text)
    r5, g6, b5 = r >> 3, g >> 2, b >> 3
    return '#%02x%02x%02x' % (r5 << 3 | r5 >> 2, g6 << 2 | g6 >> 4, b5 << 3 | b5 >> 2)


def wrap(draw, text, font, width):
    lines, line = [], ''
    for word in text.split():
        test = f'{line} {word}'.strip()
        if draw.textlength(test, font=font) <= width:
            line = test
        else:
            lines.append(line)
            line = word
    if line:
        lines.append(line)
    return lines


def main():
    width, pad = 1500, 40
    swatch_w, swatch_h = 150, 56
    text_x = pad + swatch_w + 250
    text_w = width - text_x - pad
    probe = ImageDraw.Draw(Image.new('RGB', (10, 10)))
    rows = []
    y = 130
    for group, colours in GROUPS:
        rows.append(('head', group, y))
        y += 46
        for hex_text, name, where in colours:
            lines = wrap(probe, where, TEXT, text_w) if where else []
            height = max(swatch_h, 26 + 24 * len(lines)) + 12
            rows.append(('row', (hex_text, name, lines), y))
            y += height
        y += 18
    rows.append(('head', 'Bliske boje (za pregled ekran po ekran)', y))
    y += 46
    for near in NEAR:
        rows.append(('near', near, y))
        y += 58
    total = y + pad
    image = Image.new('RGB', (width, total), BACK)
    draw = ImageDraw.Draw(image)
    draw.text((pad, 34), 'RUZINA — paleta ekrana na ploči', font=TITLE, fill=INK)
    draw.text((pad, 82), 'Boje iz overview.slint i iz Rust crtanja (slint_ui.rs). Desno od uzorka: kako je zapisano → kako je panel prikaže (RGB565).',
              font=TEXT, fill=MUTED)
    for kind, data, top in rows:
        if kind == 'head':
            draw.text((pad, top + 6), data.upper(), font=HEAD, fill='#e64c0c')
            draw.line((pad, top + 38, width - pad, top + 38), fill=LINE, width=1)
        elif kind == 'row':
            hex_text, name, lines = data
            draw.rounded_rectangle((pad, top, pad + swatch_w, top + swatch_h), radius=6, fill=hex_text, outline=LINE)
            draw.text((pad + swatch_w + 16, top + 4), hex_text, font=MONO, fill=INK)
            shown = panel(hex_text)
            draw.text((pad + swatch_w + 16, top + 30), f'→ {shown}', font=SMALL, fill=MUTED if shown != hex_text else '#574b45')
            draw.text((text_x, top + 2), name, font=HEAD.font_variant(size=19), fill=INK)
            for index, line in enumerate(lines):
                draw.text((text_x, top + 28 + 24 * index), line, font=TEXT, fill=MUTED)
        else:
            *colours, note = data
            x = pad
            for hex_text in colours:
                if hex_text is None:
                    continue
                draw.rounded_rectangle((x, top, x + 90, top + 44), radius=6, fill=hex_text, outline=LINE)
                draw.text((x, top + 46), hex_text, font=SMALL, fill=MUTED)
                x += 104
            draw.text((pad + 3 * 104 + 20, top + 10), note, font=TEXT, fill=INK)
    out = Path(__file__).with_name('ruzina-screen-palette.png')
    image.save(out)
    print(out, image.size)


if __name__ == '__main__':
    main()
