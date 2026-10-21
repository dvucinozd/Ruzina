JC8012P4A1C_I_W_Y
Image 8bc11e3d06a6-src-6c048e6200441927-jc8012-psram
ELF SHA256 b5497e01571b3332a1feac2a5580bd3e47f0e2b2e91df7069c159984095c1b3f
Diagnostic probe: psram
BUILD ONLY / NOT FLASHED / NOT QUALIFIED
PSRAM variants initialize external RAM at 80 MHz and overwrite only a reserved 64 KiB CPU scratch span.
They do not test cache writeback, DMA, full capacity or production clocks.
Preserve factory log and verified backup. Establish ROM recovery, observed flash geometry,
and a board-specific programming layout before selecting any flash command.
The historical custom-M1 partition table is not a Guition programming instruction.
Use guition_first_boot.py ingest/capture for separate unit/scenario records.
