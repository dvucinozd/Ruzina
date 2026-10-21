JC1060P470C_I_W_Y
Image 8bc11e3d06a6-src-6c048e6200441927-jc1060-none
ELF SHA256 efc084ca3348edb7c30c5ff6386b9fdd006e29593cf10da1cdbb40fc9162b0d6
Diagnostic probe: none
BUILD ONLY / NOT FLASHED / NOT QUALIFIED
PSRAM variants initialize external RAM at 80 MHz and overwrite only a reserved 64 KiB CPU scratch span.
They do not test cache writeback, DMA, full capacity or production clocks.
Preserve factory log and verified backup. Establish ROM recovery, observed flash geometry,
and a board-specific programming layout before selecting any flash command.
The historical custom-M1 partition table is not a Guition programming instruction.
Use guition_first_boot.py ingest/capture for separate unit/scenario records.
