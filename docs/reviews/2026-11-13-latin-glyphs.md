# JC1060: Croatian and other Latin letters on the deck screen

Date: 2026-11-13. Branch: `ogranak/jc1060-latin-glyphs`, from `main` at `c0f2dfc`.

## Problem

Track titles and artists with č, ć, š, ž, đ, or with é, ü, ñ and the like, did not draw on
the deck screen.

The text itself was fine: the catalog decodes rekordbox's UTF-16 strings, and the titles
reach the screen as they are (`track.title.as_str().into()`). The glyphs were missing.

Slint's software renderer draws from glyphs it embeds at build time. In 1.18 that set is:
- ASCII letters, digits and punctuation;
- every character that appears in a string literal in the `.slint` files.

Nothing else exists on the board.

## Change

An invisible, zero-size Text in the Overview holds 233 characters, so Slint embeds them in
every font size the screen uses:
- Latin-1 Supplement, U+00A1 to U+00FF, without the soft hyphen;
- Latin Extended-A, U+0100 to U+017F, which holds the Croatian letters;
- ‘ ’ ‚ “ ” „ – — • € ™.

## Cost

- The app grew from 1.80 MB to 2.45 MB (+641 KB) in the 12 MB app partition.
- Drawing on the Overview is unchanged (s91 against s87 and s88, the soak DJ playing, the
  screen untouched):
  - the average Slint frame about 4.4 ms on all three;
  - the worst frame per 10 s about 8 to 9 ms on all three.
- While the owner browsed the Library and loaded tracks, single frames reached 113 ms; s87
  showed 78 ms right after boot. A page full of new text is the costly case.

## Evidence

- s91 (app `1b61c536…`): owner verdict "Radi". Croatian and other letters draw correctly in
  the Library and in the deck titles.
- Underruns 0/0, `packet_errors=0`, no panics in the capture so far.

## Test plan

1. LIBRARY: titles and artists with č, ć, š, ž, đ, é, ü draw in full.
2. Loaded into a deck, the title bar shows them too.
3. The Overview draws as before.
