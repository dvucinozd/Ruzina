# Pajoniiir-M1 current design status B10

> Historical custom-PCB snapshot. The [2026-10-20 platform decision](M1_GUITION_PLATFORM_DECISION_2026-10-20.md) discontinues custom PCB development and selects Guition JC1060P470C_I_W_Y and JC8012P4A1C_I_W_Y for native Rust development. The milestones and open manufacturing gates below apply to the retained custom-board design.

**Date:** 2026-09-08

**Electrical milestone:** M1-ELEC-B2

**Mechanical milestone:** M1-MECH-B7

**Pre-layout milestone:** M1-PRELAYOUT-B10

**Layout state:** populated board-first canvas with routed P4 flash, crystal and external-DCDC feasibility proof; `layout_freeze_allowed=false`

**ECAD toolchain:** KiCad 10.x only for all future edits and CI

## Product boundary

Pajoniiir-M1 is the primary product assembly: a standalone ESP32-P4 mainboard. Its PCB outline, mounting holes, connector locations, height zones and enclosure datums must come from the board's own packing, routing, thermal, service and manufacturing requirements.

The screen is a replaceable external peripheral. DSI506 / DYL0023 remains one electrically and physically qualified compatibility profile; it does not define the mainboard outline or mounting pattern.

## Electrical and PCB state

The hierarchical KiCad design contains 15 leaf sheets. Current structural validation passes, all 244 assigned footprints are on the board, and the board retains 193 named schematic nets. The intentional blank-footprint set remains J1, C3 and C8.

The board retains:

- ESP32-P4 with ESP32-C6 over four-bit SDIO;
- independent USB host ports and switched VBUS branches;
- PCM5102A stereo MAIN output;
- native four-bit SDMMC microSD;
- power protection, current measurement and power-good diagnostics;
- generic 15-pin Raspberry-Pi-style MIPI DSI host interface.

B8 populated the eight reversible `Dwgs.User` electrical domains. B9 compacted the 63 P4 and flash-sheet footprints into a deterministic island. B10 rotates U1 and adds real copper for the external QSPI flash, 40 MHz crystal and external VDD_HP regulator.

The B10 live board contains 78 track segments and 11 vias. There are no footprint bounding-box overlaps in the P4 group, no zones and no `Edge.Cuts`. The crystal body is 5.203 mm from U1. The six QSPI paths have a 12.208 mm screening length spread and three package-opposed paths use one via pair each.

KiCad 10.0.4 reports zero route-specific geometry violations. U8 now uses a project-local KiCad 10 footprint with four 0.30 mm thermal-via drills, clearing the previous minimum-drill conflict. The unfinished whole board still has 499 unconnected items and nine non-routing findings: eight silkscreen findings and the intentionally absent outline.

`m1_prelayout_b10_core_routes.json` is the current machine-readable route authority. B9 remains the reproducible unrouted source placement.

## Display interface

J6 remains Amphenol `SFW15R-2STE1LF`, 15 contacts at 1.0 mm pitch, top contact, right-angle SMT ZIF. It routes DSI clock, lane 0, lane 1, shared display I2C and `3V3_DISPLAY_MODULE`.

Every supported display model still needs its own pin-map or adapter, power envelope, DSI timing/controller profile, touch behavior and cable orientation/service qualification. Type-B is a DSI506 cable fact, not a universal M1 requirement.

## Locked and open items

Locked today:

- electrical connectivity and current 15-sheet hierarchy;
- J6 part, footprint and 15-pin electrical map;
- JLCPCB `JLC04161H-7628` four-layer, 1.6 mm stackup;
- most external connector MPN and footprint intent;
- DSI506 compatibility profile;
- reproducible B9 source placement and B10 critical-route proof;
- U8 project-local VQFN thermal-via variant with 0.30 mm drills;
- 0.15 mm Default clearance for current fine-pitch escape validation;
- `layout_freeze_allowed=false` while blockers remain.

The 12 layout blockers remain:

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

## Next board work

1. Place U4 at a candidate board edge, enforce its all-layer antenna keepout and prove the four-bit SDIO escape to U1.
2. Add the complete P4 power/GND plane and stitching strategy.
3. Compact the power spine and connector domains around the P4 and C6 islands, then derive the minimum board-owned envelope.
4. Add a chassis mounting pattern and validate cables, height zones, thermal clearance and user access.
5. Close J1/C3/C8 EVT and exact controlled-impedance geometry before final route freeze.

No Gerber or EVT order is authorized while the outline and blocking gates remain open.

## Validation

```powershell
python hardware/Pajoniiir-M1/tools/validate_schematic_structure.py
python hardware/Pajoniiir-M1/tools/validate_mechanical_authority.py
python hardware/Pajoniiir-M1/tools/validate_board_packing_inventory.py
python hardware/Pajoniiir-M1/tools/report_mech_gate_snapshot.py
kicad-cli pcb drc --format json --output <b10-drc.json> `
  hardware/Pajoniiir-M1/Pajoniiir-M1.kicad_pcb
& 'C:\Program Files\KiCad\10.0\bin\python.exe' `
  hardware/Pajoniiir-M1/tools/validate_core_routing_b10.py `
  --board hardware/Pajoniiir-M1/Pajoniiir-M1.kicad_pcb `
  --project hardware/Pajoniiir-M1/Pajoniiir-M1.kicad_pro `
  --report hardware/Pajoniiir-M1/m1_prelayout_b10_core_routes.json `
  --drc <b10-drc.json>
```

The B8 and B9 validators apply to their historical unrouted artifacts. The B10 validator is the live-board routing gate.
