# Guition hardware matrix and Rust BSP preparation

Date: 2026-10-20

Scope: complete source-based preparation for the two selected models while the boards are in transit. All hardware values below are documented candidates. No received unit has been interrogated, flashed or physically accepted. The owner's v3.2 report remains separate from schematic/package labels and vendor demo folder names.

## Source authority

The owner supplied the current vendor packages in:

- `C:/Users/Daniel/Desktop/ESP-BOARDS/JC1060P470C_I_W_Y`
- `C:/Users/Daniel/Desktop/ESP-BOARDS/JC8012P4A1C_I_W_Y`

These packages take precedence over older public demos for this preparation. Selected immutable file hashes and relative paths are recorded in [source-manifest.json](source-manifest.json). No vendor executables were run. Vendor code, firmware payloads and panel tables were inspected only in the external research directory and are not imported into the Rust source tree.

The public comparison baseline is [guitionofficial/P4-series at 9f151095](https://github.com/guitionofficial/P4-series/tree/9f1510954d281e30b19cd8baf115e2ed2a65a90a). Its older JC8012 schematic PNGs are secondary to the local seven-page PDF. The vendor's [model-selection table](https://www.guition.com/model-selection) incorrectly lists 1024x600 for JC8012; the supplied specification p3/p4 and actual JD9365 BSP branch establish the documented 800x1280 candidate.

Local source keys used below:

| Key | Relative source inside the relevant owner package |
|---|---|
| 1060-SCH | `5-Schematic/JC1060P470C_I_W_Y-V1.0.pdf`, one sheet, visually reviewed |
| 1060-SPEC | `2-Specification/JC1060P470C_I_W_Y Specifications-EN.pdf`, p3/p4 |
| 1060-V3 | `1-Demo/P4V3X_DEMO/IDF_6.1.0(P4V3_V3.8)/common_components/espressif__esp32_p4_function_ev_board/`, BSP header, C implementation and `include/bsp/display.h` |
| 8012-SCH | `5-Schematic/JC8012P4A1.pdf`, seven sheets; p3 core, p6 USB, p7 codec visually reviewed |
| 8012-SPEC | `2-Specification/JC8012P4A1C_I_W Specifications-EN-v1.1(2).pdf`, p3/p4 |
| 8012-V3 | `1-Demo/P4V3X_demo/IDF_6.0.2/JC8012P4A1C_I_W_Y_NewPanel(P4V3x)/common_components/esp32_p4_function_ev_board/`, BSP header, C implementation and `include/bsp/display.h` |

The IDF version labels describe vendor examples, not Rust dependencies. This project stays on its pinned native Rust toolchain/HAL.

The recorded local hashes can be replayed from the repository root with `python firmware-rust/tools/check_board_sources.py --base C:/Users/Daniel/Desktop/ESP-BOARDS`. Public comparison records are immutable commit/blob references; this command checks the supplied local packages only.

## Hardware matrix

| Area | JC1060P470C_I_W_Y | JC8012P4A1C_I_W_Y | Evidence / remaining check |
|---|---|---|---|
| Silicon | Owner reports ESP32-P4 v3.2 | Owner reports ESP32-P4 v3.2 | Read actual chip revision after arrival; minimum remains 302 |
| Memory | 16 MiB external flash; 32 MiB PSRAM | 16 MiB external flash; 32 MiB PSRAM | SPEC p3 and QIO/16MB/200MHz PSRAM demo configs; identify actual devices/mode before storage writes or allocator activation |
| Display | 7 inch, native 1024x600, JD9165 | 10.1 inch, native 800x1280, JD9365 | SPEC p4, actual panel constructors in V3 BSP; landscape UI rotation is a separate choice |
| DSI | 2 lanes; PHY LDO3 at 2.5 V | 2 lanes; PHY LDO3 at 2.5 V | V3 display headers; reset/init and cache/DMA remain untested |
| Panel reset / backlight | GPIO0 / GPIO23 | GPIO27 / GPIO23 for 800x1280 branch | V3 BSP headers; choose exact panel variant before enabling |
| Touch | GT911 | GSL3680; vendor wrapper uses address 0x40 | V3 constructor branches; exact address/orientation/calibration still requires unit interrogation |
| Shared I2C | SDA7 / SCL8 | SDA7 / SCL8 | V3 headers and schematic codec/touch nets |
| Touch reset / interrupt | Schematic shows 22/21; V3 example sets NC/NC | 22/21 in 800x1280 branch | Conflict retained for 1060; do not drive the pins by inference |
| SDMMC | Slot0, D0..3=39..42, CLK43, CMD44 | Same | V3 headers and TF schematic |
| SD power / detect | P4 internal LDO4; no established GPIO card-detect | Same | V3 `ldo_chan_id=4`, schematic `ESP_LDO_VO4`; do not reuse M1 GPIO45 power or GPIO46 detect |
| Wireless | ESP32-C6 companion in Guition core/module | ESP32-C6; schematic SDIO2 on P4 GPIO14..19 and reset/control connections | Unit/module revision and companion firmware remain open; no Wi-Fi policy or companion image imported |
| Native USB | Separate FS and HS Type-C data connectors | Separate FS and HS Type-C data connectors | SCH; JC8012 also has CH340C USB/UART service connector |
| USB host power | Shared `USB5V_IN`, Type-C CC pulldowns; no independent M1-style VBUS switch/fault channels established | Same | Host adapters, supply/backfeed policy and current limits are unqualified |
| FS GPIO binding | Unresolved for exact v3.2 module/board | Unresolved for exact v3.2 board | Old raw schematic labels 24/25 and copied BSP USB macros 19/20 must not be translated into a working v3.2 HAL binding by assumption |
| Audio codec | ES8311, differential mono speaker path | ES8311, differential mono speaker path | SCH and vendor mono I2S configuration; not a stereo PCM5102A MAIN replacement |
| Audio I2S | MCLK13, BCLK12, LRCK10, DOUT9, DIN48; PA enable11 | MCLK13, BCLK12, LRCK10, DOUT9, DIN11; PA enable20 | V3 BSPs; physical sample-rate/clock/mute tests remain pending |
| Stereo MAIN / headphone | No qualified onboard stereo MAIN or independent headphone path | Same | MAIN prefers a ready external stereo DAC, then compatible controller MAIN audio. Headphones always use controller UAC; hardware wiring, detection and channel mappings remain unqualified. See [routing policy](../M1_RUST_PRODUCT_REQUIREMENTS.md#8-controller-only-headphone-policy). |
| Other hardware | Ethernet PHY/RJ45, RS485/UART, camera, RTC and battery circuitry shown | Camera, RTC, battery circuitry and expansion FPCs shown | Keep inactive until needed and individually mapped; schematic presence is not unit acceptance |

## Panel and revision conflicts

Both supplied packages contain old/new panel variants and special P4V3X demos. A silicon revision does not identify the fitted panel revision. No init table is selected in either Rust profile.

- JC1060's V3 BSP implementation specifies a 52 MHz DPI clock, 750 Mbps per lane, H sync/back/front 24/136/160 and V sync/back/front 2/21/12 for its 1024x600 path. Its generic display header contains different timing macros and a 1000 Mbps lane macro; an Arduino `pins_config.h` in the new-panel folder still declares 480x800. The implementation/specification, rather than those generic/template labels, supplies the candidate geometry. Exact panel behavior is open.
- JC8012's V3 implementation specifies 70 MHz, H sync/back/front 20/20/40 and V sync/back/front 4/10/30 for the JD9365 path. This is an initialization candidate only. Earlier public JD9365 tables are not automatically interchangeable with this bundle or the arriving units.
- The local JC8012 PDF metadata was generated in 2026, but the internal schematic title blocks identify V1.0 with 2024 dates. Its raw GPIO labels therefore do not prove a revised silicon-package map. The model/folder name also does not prove it.

GSL3680 firmware is not bundled. The current published [Guition touch NOTICE](https://github.com/Guition-JC/Guition/blob/10427a8450f6ffdddac062bcea2fd258a61a5a8d/components/touch/esp_lcd_touch_gsl3680/NOTICE) explicitly excludes the retained controller payload from its MIT licence. The local payload/tracking sources also have no established distribution clearance in this audit. A future driver needs a documented payload/provisioning source before importing it. This does not prevent the present data-only BSP preparation.

## Rust BSP structure

`pajoniiir-board-profile` is a `no_std`, HAL-independent crate with separate `jc1060` and `jc8012` modules. It records documented capacities, display/controller identity, candidate GPIOs, LDO requirements and unresolved USB/panel state. `check_unit` rejects old silicon and memory variants; success only compares detected identity and does not qualify or activate peripherals.

The embedded target has three mutually exclusive selectors:

- `board-jc1060`: JC1060 identity and scheduler-only BSP skeleton.
- `board-jc8012`: JC8012 identity and scheduler-only BSP skeleton.
- `board-legacy-m1`: historical custom-board tasks and wiring isolated in `src/platform/legacy_m1.rs`.

The default remains `default = []`: generic P4 idle/scheduler target, with no board selected. Legacy USB/SD/flash/PSRAM features now require explicit `board-legacy-m1`. They cannot be combined with a Guition selector. No Guition GPIO is converted into a HAL peripheral handle in this preparation. No Guition flash layout is selected, and the existing `espflash.toml`/Hot Cue partitions remain legacy-only assets, not Guition flashing instructions.

Compile examples, from `firmware-rust/apps/m1-firmware`:

```sh
cargo +1.95.0 build --locked --release --features board-jc1060
cargo +1.95.0 build --locked --release --features board-jc8012
cargo +1.95.0 check --locked --features board-legacy-m1,render-graph-bringup,sdmmc-profile-storage-bringup
python ../../tools/check_board_selection.py
```

CI retains all existing legacy feature configurations under explicit board selection, adds independent Guition compile/link gates, and rejects ambiguous selectors plus every direct legacy hardware feature without a legacy board. Dependency locks change only to register the new local board-profile crate; external dependency versions and silicon policy are unchanged.

## First work after arrival

Record photos/PCB and panel markings, detected chip revision, flash identity/capacity/mode, PSRAM size and the factory firmware identity before choosing a hardware profile. Resolve the USB-FS package/pad mapping and host-power wiring before connecting a controller or memory stick as a host. Then implement one peripheral adapter at a time: memory, DSI/backlight/touch, SD, USB and an agreed audio-output path. Acceptance is per exact board and flashed image. APTA remains deferred.
