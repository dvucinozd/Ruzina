# New_Panel table provenance

`main/new_panel_commands.h` retains the ordered command/data/length/delay values
from the owner-supplied Guition `JC1060P470C_I_W_Y_New_Panel` BSP:
`common_components/espressif__esp32_p4_function_ev_board/esp32_p4_function_ev_board.c`.
The source has SPDX Apache-2.0 headers and its adjacent LICENSE is preserved
here as `Guition-BSP-Apache-2.0.txt`. Source file SHA-256:
`ff56c06c05a6dea121d2a393566f8cd6c961cfb14a997fe753421903fed7bcb4`.
The source's normalized table SHA is recorded in the repository panel manifest.
Only the60 command records were adapted to fixed-array storage; no vendor
driver/BSP implementation was imported. Driver calls use the pinned IDF6.1.
This does not assert the vendor's transitive provenance beyond its supplied
license, nor physical panel identification by the stale EK79007 log message.
