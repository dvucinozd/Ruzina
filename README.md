# Pajoniiir-M1

**Pajoniiir standalone dual-deck DJ system** — native Rust development on Guition ESP32-P4 boards.

## Active direction — 2026-10-20

Latest mixer/FX work: [authoritative mixer/PFL and Beat/Pad FX controls](docs/M1_RUST_MIXER_FX_UI_GATE_2026-10-21.md) in both host UI geometries, with momentary release/recovery tests and 24 reviewed pixel goldens. Audible FX execution and physical output remain open.

Latest UI work: [interactive deck/Library/cue/session-settings simulator](docs/M1_RUST_INTERACTIVE_UI_GATE_2026-10-21.md) uses authoritative deck state, imported waveforms and reviewed screenshots for both Guition geometries. Full UI parity and physical board/audio qualification remain open.

Latest analysis work: [pinned Rekordbox import and deck consumers](docs/M1_RUST_REKORDBOX_ANALYSIS_GATE_2026-10-21.md), isolated from the open audio PR.

Latest software gate: [completed-output position and EOF](docs/M1_RUST_PLAYBACK_PROGRESS_GATE_2026-10-21.md), integrated in the reference target and canonical host simulation. Physical I2S/UAC completion and clock qualification remain open.

Latest software follow-up: [consumer-clock PCM delivery and simulation](docs/M1_RUST_SINK_DELIVERY_GATE_2026-10-20.md). Physical audio drivers and exact-board qualification remain pending.

Custom M1 PCB development has been discontinued by project decision. The selected development boards are **JC1060P470C_I_W_Y** and **JC8012P4A1C_I_W_Y**, both with **ESP32-P4 silicon v3.2**, as reported by the project owner. Exact unit identity and board peripherals still require bring-up evidence.

Native Rust development now uses `main` after [PR #1](https://github.com/dvucinozd/Pajoniiir-M1/pull/1). Its `ESP_HAL_CONFIG_MIN_CHIP_REVISION=302` matches the selected silicon revision. Firmware sources are in `firmware-rust/`; the separate `codex/native-apta-consumer` branch is retained and is not part of this integration. See the [integration validation](docs/M1_RUST_MAIN_INTEGRATION_2026-10-20.md) for tests and remaining gates.

See [Guition platform decision and migration plan](docs/M1_GUITION_PLATFORM_DECISION_2026-10-20.md) for scope, current evidence and the next development steps.

Source-based board preparation is recorded in the [Guition hardware matrix](docs/boards/GUITION_HARDWARE_MATRIX_2026-10-20.md). The Rust target now has explicit, mutually exclusive `board-jc1060`, `board-jc8012` and `board-legacy-m1` selectors. Guition selectors currently run only the scheduler BSP skeleton; historical M1 hardware features require the explicit legacy selector.

The [development review after PR #4](docs/M1_RUST_DEVELOPMENT_OPEN_QUESTIONS_2026-10-20.md) records open product decisions, implementation and acceptance gates, and the recommended next work while the boards are in transit.

The [live PCM history, routing and diagnostic preparation gate](docs/M1_RUST_AUDIO_HISTORY_ROUTING_GATE_2026-10-20.md) adds the canonical Master Tempo/scratch path, host-tested DAC/controller selection with controller-only headphones, and opt-in Guition boot diagnostics. Physical audio/display/storage and exact-board acceptance remain pending.

The KiCad design, electrical contracts, mechanical evidence, libraries and validators are retained as historical engineering work. The custom-board pin map, peripheral wiring, flash layout and manufacturing gates do not define the Guition board configuration. The custom-PCB status, architecture and manufacturing sections below describe the historical baseline; native Rust development remains active.

## Historical custom-PCB status

**Electrical milestone:** M1-ELEC-B2

**Mechanical milestone:** M1-MECH-B7

**Pre-layout milestone:** M1-PRELAYOUT-B10 - routed ESP32-P4 flash, crystal and external-DCDC feasibility proof

**Schematic structure:** PASS — 15/15 leaf sheets

**KiCad toolchain:** KiCad 10 only; local KiCad 10.0.4 ERC passes with zero violations and CI installs the KiCad 10 stable release

**Manufacturing BOM parity:** PASS — 244 source / 244 exported, 15 DNP, 3 intentional blank footprints

**Board-first mechanical contract:** PASS — display profiles cannot define the PCB outline or mounts

**Display-derived B2-B6 geometry:** historical screening only

**Final placement/routing freeze:** BLOCKED — 12 physical/EVT gates remain

**Gerber/EVT order:** NOT AUTHORIZED

Evidence updates: [ERC/library closure](docs/Pajoniiir_M1_ERC_Library_Closure_2026-09-04.md), [DSI506 rear image and corrected height](docs/Pajoniiir_M1_DSI506_Evidence_2026-09-04.md).

The current human-readable snapshot is [Pajoniiir M1 Current Design Status B10](docs/Pajoniiir_M1_Current_Design_Status_B10.md).

## Architecture

```text
                         5 V LOCKING INPUT
                                |
                         TPS259474 eFuse
                                |
                         5V_PROTECTED
                                |
                    5 mOhm Kelvin system shunt
                       / INA238 measurement
                                |
                             5V_SYS
              +-----------------+------------------+
              |                                    |
          TPS62132                         USB VBUS switches
          3V3_SYS                          TPS25221 x2
              |                               |       |
      +-------+---------+                  USB0 HS  USB1 FS
      |       |         |                  storage   FLX4
      v       v         v
  ESP32-P4 ESP32-C6  PCM5102A
      |      SDIO      MAIN L/R
      |
      +-- 15-pin MIPI DSI + I2C --> qualified display profile
      +-- SDMMC -----------------> microSD
```

The mainboard exposes a generic 15-pin Raspberry-Pi-style MIPI DSI host interface. EYOYO DSI506 / DYL0023 is the first validated display profile; other models may use the same board after electrical, power, firmware and cable qualification.

## Electrical baseline

- ESP32-P4NRW32X, target silicon v3.2 or newer approved revision
- 32 MB in-package PSRAM
- W25Q128JVPIQ 16 MB external QSPI flash
- ESP32-C6-WROOM-1-N4 over four-bit SDIO
- USB0 dedicated High-Speed storage host
- USB1 GPIO26/27 Full-Speed DDJ-FLX4 MIDI/UAC host
- independent TPS25221 VBUS switching, about 1.0 A USB0 and 1.6 A USB1 initial limits
- PCM5102APWR stereo MAIN output with deterministic XSMT boot mute
- native four-bit SDMMC microSD with TPS22918 power cycling
- P4 UART, P4 USB Serial/JTAG pogo and C6 recovery paths
- optional INA238 system power telemetry

## DSI host contract

```text
Host connector    Amphenol SFW15R-2STE1LF
Interface         15 contacts, 1.0 mm, Raspberry-Pi-style DSI map
Supply branch     3V3_DISPLAY_MODULE; final multi-model budget open
DSI routing       clock + lane0 + lane1
I2C               GPIO7 SDA / GPIO8 SCL
Cable             qualified per display model or adapter
Validated profile DSI506 / DYL0023: lane0, 800 Mbps, RGB888, 27.777 MHz
```

Connector shape alone does not prove compatibility. Every supported display needs a recorded pin map, power envelope, DSI timing/controller profile and cable orientation.

## Mechanical baseline

The mainboard is a standalone assembly and mounts to its own chassis or enclosure bosses. Its outline, mounting pattern and connector coordinates remain open until complete footprint packing, routing, thermal and service-envelope work establishes a viable board.

DSI506-derived values such as the 58 x 49 mm rear posts, 104 x 62 mm board screen and 128 x 84 x 30 mm enclosure screen are retained only for optional DSI506 bracket/enclosure work. They are not production `Edge.Cuts` authority.

## Connector and footprint state

| RefDes | Production part | Footprint | Mechanical state |
|---|---|---|---|
| J1 | Switchcraft 722RAHLP | open | terminal-center and panel geometry required |
| J2/J3 | Amphenol 87520-1010ALF | locked | panel/cable envelope open |
| J4/J5 | Kycon KLPX-0848A-2-W-G / -R-G | locked | panel/cable envelope open |
| J6 | Amphenol SFW15R-2STE1LF | locked | board-edge placement, power budget and per-display cable profile open |
| J7 | Molex 503398-1892 | locked | slot/access/clearance open |
| SW1/SW2 | B3U-3000P-B | locked | recessed tool-hole placement open |
| J9 | project-local factory pogo | closed, DNL | normal placement work only |

Only `J1`, `C3` and `C8` intentionally have blank footprints in the manufacturing source.

## PCB and routing state

`hardware/Pajoniiir-M1/Pajoniiir-M1.kicad_pcb` is a populated four-copper-layer pre-layout canvas. It contains all 244 assigned footprints and 193 named schematic nets in eight `Dwgs.User` working domains. B10 adds 78 track segments and 11 vias for the P4 external flash, crystal and external-DCDC proof. The P4 group has no footprint bounding-box overlaps; the board still has no zones or `Edge.Cuts`.

`m1_board_packing_inventory_b7.json` records all 247 schematic components, 244 assigned footprints, 33 footprint classes including the blank-gate class, and the seven board-first packing phases.

`m1_board_placement_seed_b8.json` records the generated domain coordinates and explicitly marks every coordinate as non-production. The seed and validator scripts preserve schematic UUID paths so KiCad can continue synchronizing the board with the hierarchy.

`m1_prelayout_b9_core_island.json` remains the reproducible unrouted source placement. `m1_prelayout_b10_core_routes.json` records the live critical routes and metrics. Its validator enforces the 5.203 mm U1/Y1 body gap, QSPI transition topology, 0.60/0.30 mm B10 vias, power-trunk widths, zero P4 bounding-box overlaps and zero route-geometry DRC violations.

The stackup is locked to JLCPCB `JLC04161H-7628`, 1.6 mm, 1 oz outer copper and 0.5 oz inner copper.

```text
F.Cu   components + critical USB/MIPI/QSPI/SDIO routing
In1.Cu continuous solid GND reference
In2.Cu power distribution + compatible low-speed routing
B.Cu   secondary components and low-speed routing
```

B5 locks routing topology and provides screening geometry:

- USB: 90 ohm differential; screening 0.2332 mm width / 0.15 mm gap
- MIPI: 100 ohm differential; screening 0.1722 mm width / 0.15 mm gap

These numbers are not production impedance locks. Exact JLCPCB calculator records, soldermask/model selection and matching KiCad rules are required before route freeze.

## Open layout blockers

The current 12 blocking gates are:

```text
C3_INPUT_BULK
C8_PROTECTED_BULK
J1_POWER_INPUT
SW1_RESET
SW2_BOOT
J2_USB0
J3_USB1
J4_RCA_L
J5_RCA_R
J_LCD_DISPLAY_FPC
J7_MICROSD
PCB_OUTLINE
```

Although most connector MPNs and footprints are locked, their gates stay open until absolute panel datums, cutouts, mating envelopes and local clearances are physically validated.

## Repository structure

```text
docs/                         engineering and status documents
hardware/Pajoniiir-M1/        KiCad project and machine contracts
  *.kicad_sch                 root plus 15 hierarchical sheets
  Pajoniiir-M1.kicad_pcb      populated four-layer B10 critical-route canvas
  *.json                      electrical/mechanical/freeze authorities
  libraries/                  project symbols and footprints
  tools/                      migration and fail-closed validators
.github/workflows/            KiCad and mechanical CI
```

## Authority order

When sources disagree:

1. live `hardware/Pajoniiir-M1/*.kicad_sch`
2. `board_first_mechanical_contract.json` and `mechanical_gates.json`
3. `pcb_constraints.json` and the current routing contract
4. B4 exact connector source lock
5. display compatibility profiles, beginning with `display_compatibility_dsi506.json`
6. [Engineering BOM v0.3](docs/Pajoniiir_Mainboard_BOM_v0.3.md)
7. [Global GPIO allocation](docs/Pajoniiir_Global_GPIO_Allocation_v0.1.md)
8. subsystem documents
9. superseded display-mounted B2-B6 and JC4880 records as historical evidence

## Current documents

### Status and release gates

- [Development handoff and remaining work](docs/Pajoniiir_M1_Development_Handoff_2026-09-08.md)
- [Current Design Status B10](docs/Pajoniiir_M1_Current_Design_Status_B10.md)
- [B10 P4 Critical Routes](docs/Pajoniiir_M1_PRELAYOUT_B10_P4_Critical_Routes_v0.1.md)
- [B9 P4 Core Island](docs/Pajoniiir_M1_PRELAYOUT_B9_P4_Core_Island_v0.1.md)
- [Schematic Audit](docs/Pajoniiir_M1_Schematic_Audit_v0.1.md)
- [Schematic Readiness Review](docs/Pajoniiir_RevA_Schematic_Readiness_Review_v0.1.md)
- [Manufacturing Output Contract](docs/Pajoniiir_Manufacturing_Output_Contract_v0.1.md)
- [Mechanical and Sourcing Gates](docs/Pajoniiir_M1_Mechanical_Sourcing_Gates_v0.1.md)
- [PCB Placement and Routing Constraints](docs/Pajoniiir_M1_PCB_Layout_Constraints_v0.1.md)

### Electrical contracts

- [Engineering BOM v0.3](docs/Pajoniiir_Mainboard_BOM_v0.3.md)
- [Global GPIO Allocation](docs/Pajoniiir_Global_GPIO_Allocation_v0.1.md)
- [Hardware/Firmware Contract](docs/Pajoniiir_M1_Hardware_Firmware_Contract_v0.1.md)
- [5-inch DSI Interface Migration](docs/Pajoniiir_M1_ELEC_B0_5in_DSI_Interface_Migration_v0.1.md)
- [15-pin DSI Connector Lock](docs/Pajoniiir_M1_ELEC_B1_DSI15_Connector_Lock_v0.1.md)
- [Board-first mechanical rebase](docs/Pajoniiir_M1_MECH_B7_Board_First_Rebase_v0.1.md)
- [DSI506 compatibility evidence](docs/Pajoniiir_M1_DSI506_Evidence_2026-09-04.md)

### Historical and subsystem design records

The original architecture, schematic plan, JC4880 display/backlight, GT911 and M1-MECH-A documents remain in `docs/` as design provenance. Their display-specific geometry does not override the B7 board-first authority.

## Validation

From the repository root:

```bash
python hardware/Pajoniiir-M1/tools/validate_schematic_structure.py
python hardware/Pajoniiir-M1/tools/validate_mechanical_authority.py
python hardware/Pajoniiir-M1/tools/report_mech_gate_snapshot.py
"C:/Program Files/KiCad/10.0/bin/python.exe" hardware/Pajoniiir-M1/tools/validate_core_routing_b10.py \
  --board hardware/Pajoniiir-M1/Pajoniiir-M1.kicad_pcb \
  --project hardware/Pajoniiir-M1/Pajoniiir-M1.kicad_pro \
  --report hardware/Pajoniiir-M1/m1_prelayout_b10_core_routes.json \
  --drc <fresh-kicad-drc.json>
```

Native KiCad 10 CI loads every schematic, exports and cross-checks the manufacturing BOM, exports the hierarchy netlist/PDF, enforces ERC cleanliness and validates the B10 PCB routing contract against a fresh DRC report.

Final placement, routing, Gerbers and EVT ordering remain blocked until every `blocks_layout_freeze` gate is closed and the production impedance geometry is recorded.

## Native Rust firmware foundation

The M1 software rewrite is developed in `firmware-rust/`. The released Pajoniiir M2.2 firmware is the functional/product golden reference. Exact Guition schematics and board profiles will define the selected boards' electrical integration; the retained custom-PCB contracts apply only to the historical custom board.

Current Rust architecture decisions:

- native `no_std` ESP32-P4 product direction;
- Slint for UI;
- dedicated RGB565 Rust waveform renderer with embedded-graphics for suitable primitives;
- data-driven multi-controller profiles;
- provider-neutral TrackAnalysis for Rekordbox/APTA;
- host regression and desktop simulation, followed by separate bring-up on each selected Guition board.

References:

- [M1 Rust Product Requirements](docs/M1_RUST_PRODUCT_REQUIREMENTS.md)
- [M1 Rust Architecture](docs/M1_RUST_ARCHITECTURE.md)
- [Architecture Decision Records](docs/adr/)
- [Rust workspace](firmware-rust/README.md)

Rust host/simulator success does not close any physical PCB, USB, audio, DSI, PSRAM/DMA, C6 or power qualification gate.

