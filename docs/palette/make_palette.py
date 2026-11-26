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

# (group, [(hex, name, where)]) — as of the GUI pass, 2026-11-26 (s229).
GROUPS = [
    ('Pozadine i površine', [
        ('#000000', 'Glavna pozadina (crna)', 'ekran, lane waveforma, mini waveformi, kartice decka, rail, FX stupac, sve stranice, boot, screensaver i UPDATING; neupaljeni gumbi i oznake'),
        ('#202020', 'Tamnosiva', 'phase strip i oznaka „+12 ms”, zaglavlje Library, crte između redova u Library'),
        ('#101010', 'Gotovo crna', 'svaki drugi red popisa na Settings'),
        ('#1e1511', 'Smeđa površina', 'neodabrani izbori na Settings, gumb „FX” na FX panelu kad je ugašen'),
        ('#2a1b14', 'Aktivni rail / meter ugašen', 'pozadina aktivne ikone na railu; ugašeni segment metera'),
        ('#0b2614', 'Tamno zelena', 'MT uključen, odabrani red u Library, FX ON pritisnut'),
        ('#2a2300', 'Tamno žuta', 'Q uključen'),
        ('#301c00', 'Sjena loopa', 'raspon loopa iza waveforma'),
    ]),
    ('Linije i rubovi', [
        ('#38251e', 'Rub', 'rubovi kartica i FX gumba, linija raila, razdjelnik Hot Cues; beat linije; odsvirani dio mini waveforma'),
        ('#574b45', 'Prigušeni rub', 'neupaljeni MT/Q, BACK, BPM filter, sortiranje, CLOSE i FX rubovi; bar linije; status EMPTY'),
        ('#404040', 'Sredina phase stripa', 'crta između polovica phase stripa'),
    ]),
    ('Tekst', [
        ('#f5f0ed', 'Glavni tekst', 'naslovi, proteklo vrijeme, Library redovi, vrijednosti; memory cue linije, bijeli vrhovi i visoki u waveformu, bijela sredina phase stripa'),
        ('#a3958e', 'Sekundarni tekst', 'preostalo vrijeme, oznake, zaglavlje Library, neupaljeni gumbi, izreka na screensaveru'),
        ('#100c08', 'Tamni tekst na punim gumbima', 'D1/D2, puni PLAY/CUE/LOAD/BACK, odabrani FX izbori'),
    ]),
    ('Decki i naglasci', [
        ('#e64c0c', 'Deck 1 / Rust narančasta', 'D1, LOAD D1, DECK 1 na FX, pruga D1, aktivni rail, BPM, naslovi Settings, crta ispod zaglavlja Library, BPM filter'),
        ('#00e5ff', 'Deck 2 / cijan', 'D2, LOAD D2, DECK 2 na FX, pruga D2, SYNC na D2; Hot Cue B'),
        ('#b7410e', 'Hrđa', 'odabrani izbor na Settings, autor izreke na screensaveru'),
        ('#ff7700', 'Narančasta', 'LOOP oznaka i LOOP u memory cueovima, oznaka foldera, key nepoznat; rubovi loopa'),
    ]),
    ('Stanja', [
        ('#00ff66', 'Zelena', 'PLAY/PAUSE, MT, key oznaka, odabrani red, BACK, FX ON, key pristaje; Hot Cue A'),
        ('#ffd600', 'Žuta', 'CUE, pitch, Q, sortiranje (gumbi i zaglavlje), faza; key +2; Hot Cue D'),
        ('#ff0033', 'Žarko crvena', 'playhead i mini playhead, D1 u phase stripu, preostalo vrijeme u zadnjih 30 s, UPDATING FIRMWARE, vrh metera; Hot Cue C'),
        ('#5c1020', 'Prigušena crvena', 'preostalo vrijeme u zadnjih 30 s dok trepće ugašeno'),
        ('#ff2e63', 'Crvena brisanja', 'DELETE na Hot Cues'),
        ('#0088ff', 'Plava', 'key +7'),
    ]),
    ('Waveform (rekordbox 3-band)', [
        ('#2f6bff', 'Niski', 'bjelina 0–2; isto kao Hot Cue G'),
        ('#e0a040', 'Srednji', 'bjelina 3–4'),
        ('#f5f0ed', 'Visoki', 'bjelina 5–7'),
        ('#39ff14', 'Bar trokut', 'zeleni trokut na svakoj bar liniji'),
    ]),
    ('Phase strip', [
        ('#ff0033', 'Deck 1', 'udarci D1 u gornjoj polovici'),
        ('#0066ff', 'Deck 2', 'udarci D2 u donjoj polovici'),
    ]),
    ('Meteri', [
        ('#39ff14', 'Do 0 dB', 'acid zelena'),
        ('#e6193c', 'Iznad 0 dB', 'koraljno crvena (u kodu METER_AMBER)'),
        ('#ff0033', 'Vrh', 'žarko crvena'),
    ]),
    ('Hot Cue A–H', [
        ('#00ff66', 'A', 'linija i zastavica na velikom i mini waveformu, pločica na Hot Cues'),
        ('#00e5ff', 'B', ''),
        ('#ff0033', 'C', ''),
        ('#ffd600', 'D', ''),
        ('#a259ff', 'E', ''),
        ('#ff2ec4', 'F', ''),
        ('#2f6bff', 'G', ''),
        ('#b6ff00', 'H', ''),
    ]),
    ('FX efekti', [
        ('#eb870f', 'ECHO', 'FX panel i FX stranica: gumb efekta, odabrani BEAT i DEPTH'),
        ('#b44ae0', 'FLANGER', ''),
        ('#27c7c7', 'DELAY', ''),
        ('#2d85cd', 'FILTER i ostali', ''),
    ]),
]

# Colours close enough to confuse, for later passes.
NEAR = [
    ('#ff0033', '#ff2e63', '#e6193c', 'tri crvene: playhead, D1 faza i Hot Cue C; brisanje; iznad 0 dB'),
    ('#00ff66', '#39ff14', '#b6ff00', 'tri zelene: stanja i Hot Cue A; meter i bar trokut; Hot Cue H'),
    ('#2f6bff', '#0066ff', '#0088ff', 'tri plave: niski i Hot Cue G (iste); D2 faza; key +7'),
    ('#e64c0c', '#ff7700', '#eb870f', 'tri narančaste: Deck 1; key nepoznat i loop; ECHO'),
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
