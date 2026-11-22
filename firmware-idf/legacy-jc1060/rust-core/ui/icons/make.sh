#!/bin/sh
# Draws the page rail's icons, 26 px, white on transparent; Slint colours them (colorize).
# Run it from anywhere, with ImageMagick: sh make.sh
set -e
cd "$(dirname "$0")"
draw() {
    printf '<svg xmlns="http://www.w3.org/2000/svg" width="26" height="26" viewBox="0 0 26 26" fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">%s</svg>' "$2" |
        convert -background none -density 96 svg:- -strip "PNG32:$1.png"
}
draw overview '<path d="M2 13h4l3-8 4 16 4-12 2 4h5"/>'
draw library '<path d="M4 7h18M4 13h18M4 19h12"/>'
draw cues '<rect x="3" y="3" width="8" height="8" rx="1"/><rect x="15" y="3" width="8" height="8" rx="1"/><rect x="3" y="15" width="8" height="8" rx="1"/><rect x="15" y="15" width="8" height="8" rx="1"/>'
draw fx '<path d="M5 4v18M13 4v18M21 4v18"/><circle cx="5" cy="16" r="2.5" fill="#fff"/><circle cx="13" cy="9" r="2.5" fill="#fff"/><circle cx="21" cy="14" r="2.5" fill="#fff"/>'
draw settings '<path d="M4 7h10M18 7h4M4 19h4M12 19h10"/><circle cx="16" cy="7" r="2.5"/><circle cx="10" cy="19" r="2.5"/>'
