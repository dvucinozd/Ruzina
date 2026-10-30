# Guition peripheral ownership cards

Prepared 2026-10-27 from the [source-audited matrix](GUITION_HARDWARE_MATRIX_2026-10-20.md).
All physical ownership/binding assertions remain OPEN until measured on the unit.
Candidate pins are documentation, not authorization to drive GPIOs.

## JC1060P470C_I_W_Y

Unit-specific update2026-10-30: [S6 legacy diagnostic](../reviews/2026-10-30-legacy-s6-display-touch.md)
uses GPIO0/23 and shared I2C7/8 with New_Panel table provenance. The schematic
does connect touch21/22; this diagnostic leaves them unconfigured and polls
the existing GT911 address. It does not qualify reset/interrupt control or
transfer acceptance to the native v3.2 profiles or another unit.

| Resource | Intended owner / candidate | Evidence required before activation |
|---|---|---|
| Service/ROM transport | C01 operator, then native Serial/JTAG diagnostic owner | Exact connector, reset/ROM sequence, transport identity |
| Flash | Read-only probe first; persistent/OTA owner later | Actual RDID, geometry, backup/readback, reviewed partitions |
| PSRAM | Exclusive 64 KiB diagnostic scratch first | Actual detected map/size; allocator/cache/DMA later |
| Display / backlight | Inactive Guition panel owner; JD9165 / 1024x600 candidate; reset0, BL23 | Exact panel/init/timings, LDO3/DSI/framebuffer lifecycle |
| I2C / touch | Shared I2C owner SDA7/SCL8; GT911 candidate | Touch reset/IRQ conflict 22/21 vs NC/NC must be resolved |
| SDMMC / SD power | Media owner; slot0 D0..3=39..42, CLK43, CMD44; LDO4 | Unit wiring, safe power policy, presence/removal; no inferred detect GPIO |
| USB HS / FS | Qualified storage/controller role plan only | FS v3.2 mapping and PHY/connector roles unresolved |
| USB VBUS | Separate power policy required | Switching/current limits/backfeed/fault behavior unqualified |
| Stereo MAIN / PFL | Ready external DAC MAIN with controller fallback; PFL controller only | Exact DAC interface/pins/detection, UAC channels, mute/teardown receipts |
| C6 / shared SDIO | Network owner, inactive | Companion revision/firmware, bus/reset mapping and ownership |

## JC8012P4A1C_I_W_Y

| Resource | Intended owner / candidate | Evidence required before activation |
|---|---|---|
| Service/ROM transport | C01 operator; CH340 UART and native Serial/JTAG are distinct | Exact connector/reset/ROM sequence; actual diagnostic log path |
| Flash | Read-only probe first; persistent/OTA owner later | Actual RDID, geometry, backup/readback, reviewed partitions |
| PSRAM | Exclusive 64 KiB diagnostic scratch first | Actual detected map/size; allocator/cache/DMA later |
| Display / backlight | Inactive panel owner; JD9365 / 800x1280 candidate; reset27, BL23 | Exact new/old panel, table provenance, LDO3/DSI/framebuffer lifecycle |
| I2C / touch | Shared I2C owner SDA7/SCL8; GSL3680 candidate, reset22/IRQ21 | Actual wiring/address/orientation; payload/provisioning rights unresolved |
| SDMMC / SD power | Media owner; slot0 D0..3=39..42, CLK43, CMD44; LDO4 | Safe power policy, actual presence/removal behavior |
| USB HS / FS | Qualified storage/controller role plan only | FS v3.2 mapping and PHY/connector roles unresolved |
| USB VBUS | Separate power policy required | Switching/current limits/backfeed/fault behavior unqualified |
| Stereo MAIN / PFL | Ready external DAC MAIN with controller fallback; PFL controller only | DAC mapping, UAC channel map and physical level/listening |
| C6 / shared SDIO | Inactive network owner; vendor SDIO2 GPIO14..19 candidate | Actual module, reset/control map, bus ownership/companion firmware |

Onboard ES8311 is a differential mono speaker path, not the required stereo
MAIN or independent headphone path. Keep product owners inactive until the
specific resource's evidence closes its gate. Never transfer a donor-board
physical PASS or one Guition unit's PASS to the other model.
