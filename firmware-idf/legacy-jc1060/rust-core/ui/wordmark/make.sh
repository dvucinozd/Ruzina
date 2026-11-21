#!/bin/sh
# Cuts the owner's wordmark (firmware-rust/assets/branding/ruzina-wordmark.png) into its
# six letters, for the boot screen and the screensaver, which light them in turn. The word
# is trimmed and scaled to 720 px wide, then cut at the middle of each gap between letters,
# so that the six pictures side by side are the word again.
# Run it from anywhere, with ImageMagick: sh make.sh
set -e
cd "$(dirname "$0")"
source=../../../../../firmware-rust/assets/branding/ruzina-wordmark.png
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
convert "$source" -trim +repage -resize 720x "$work/word.png"
width=$(identify -format '%w' "$work/word.png")
# The columns with next to no ink, as runs; each run between two letters gives a cut.
cuts=$(convert "$work/word.png" -alpha extract -scale "${width}x1!" -depth 8 txt:- |
    tail -n +2 | sed -E 's/^([0-9]+),0: \(([0-9.]+).*/\1 \2/' |
    awk '{ empty = ($2 < 3); if (empty && !run) { start = $1; run = 1 }
           if (!empty && run) { print int((start + $1 - 1) / 2); run = 0 } }')
set -- $cuts
if [ $# -ne 5 ]; then
    echo "expected 5 gaps between the letters, found $#" >&2
    exit 1
fi
left=0
for letter in R U Z I N A; do
    if [ $# -gt 0 ]; then
        right=$1
        shift
    else
        right=$width
    fi
    convert "$work/word.png" -crop "$((right - left))x+$left+0" +repage -strip "$letter.png"
    left=$right
done
