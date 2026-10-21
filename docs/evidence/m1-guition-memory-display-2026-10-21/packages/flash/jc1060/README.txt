JC1060P470C_I_W_Y
Image 8bc11e3d06a6-src-6c048e6200441927-jc1060-flash
ELF SHA256 9e070f98a976e301a2dc8223a9087153ca3f99bae5e7e9152d8a51cbc8c5b417
Diagnostic probe: flash
BUILD ONLY / NOT FLASHED / NOT QUALIFIED
PSRAM variants initialize external RAM at 80 MHz and overwrite only a reserved 64 KiB CPU scratch span.
They do not test cache writeback, DMA, full capacity or production clocks.
Preserve factory log and verified backup. Establish ROM recovery, observed flash geometry,
and a board-specific programming layout before selecting any flash command.
The historical custom-M1 partition table is not a Guition programming instruction.
Use guition_first_boot.py ingest/capture for separate unit/scenario records.
