# Default artwork

`default_artwork.svg` is the operator-supplied Pajoniiir logo that M2.5 shows
when a loaded track has no usable cover. It is byte-identical to the donor's
`firmware/main-deck-p4/components/ui/assets/default_artwork.svg` (SHA-256
`cc7213894515a6438de36828c99386e7d9eebf6d5bb31bf70ea79c37bfc2661d`) and is kept
here as the source of record and for attribution.

`src/default_artwork.rs` holds the 34x34 deck and 40x40 Library RGB565 pixels
that the M2.5 tool `tools/generate_default_artwork.cjs` (sharp 0.35.4 / libvips
8.18.6) rendered from it at donor commit
`20f1c3f04a615209ae25e9bbdae649d0f5b44e8d`. The pixels are copied unchanged by
`extract_default_artwork.py` from that commit's generated
`ui_artwork_placeholder.c`:

```text
python3 -I extract_default_artwork.py ui_artwork_placeholder.c ../src/default_artwork.rs
```

No SVG renderer, decoder or file access is needed at runtime. Changing the
logo or a thumbnail size means re-rendering in the donor tool and reviewing the
UI goldens.
