# Pajoniiir-M1 development handoff

> Superseded for active development on 2026-10-20: custom PCB development has been discontinued. Native Rust development moves to Guition JC1060P470C_I_W_Y and JC8012P4A1C_I_W_Y with owner-reported ESP32-P4 v3.2 silicon. See the [platform decision](M1_GUITION_PLATFORM_DECISION_2026-10-20.md). The PCB progress estimate and B11-B16 sequence below remain historical records.

**Recorded:** 2026-09-08

**Continuation baseline:** M1-PRELAYOUT-B10 on `main`

**Supported ECAD toolchain:** KiCad 10.x only

## Overall progress

The project is approximately 35-40% complete toward a first manufacturing-ready PCB candidate. Roughly 60-65% of the PCB design work remains. This estimate ends at reviewed Gerber/BOM/PnP output for the first EVT order; physical bring-up, measurements and any board respin remain a later qualification phase.

The estimate is dominated by complete placement/routing and the unresolved mechanical inputs. Calendar time cannot be locked while the 12 physical, sourcing and enclosure gates remain unresolved.

## Completed baseline

- The hierarchical schematic contains 15 leaf sheets and passes structural validation.
- Native KiCad 10 ERC passes with zero active violations.
- The manufacturing source contains 244 assigned footprints, 15 DNP parts and three intentional blank footprint gates: J1, C3 and C8.
- All 244 assigned footprints and 193 named schematic nets are present on the board.
- The JLCPCB `JLC04161H-7628` four-layer, 1.6 mm stackup is locked.
- The product is a standalone ESP32-P4 mainboard; display modules are compatibility profiles and do not define the mainboard outline or mounts.
- B10 contains real feasibility copper for P4 external QSPI flash, the 40 MHz crystal and the external VDD_HP regulator.
- B10 contains 78 track segments and 11 vias. Its QSPI screening spread is 12.208 mm and its U1-to-Y1 body gap is 5.203 mm.
- U8 uses the reviewed project-local VQFN footprint with four 0.30 mm thermal-via drills.
- KiCad 10 DRC reports zero B10 route-geometry violations. The unfinished board has nine other findings and 499 unconnected items.
- GitHub KiCad 10 and mechanical-authority workflows pass on the current baseline.

The nine remaining DRC findings are six `silk_over_copper`, two `silk_overlap` and one `invalid_outline`. The outline finding is intentional because production `Edge.Cuts` is prohibited until the board-owned mechanical envelope is established.

## Open layout blockers

The following 12 gates still block final placement, routing freeze and fabrication release:

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

Connector MPN or footprint selection alone does not close these gates. Each relevant gate needs the final panel datum, cutout, mating envelope, cable/service clearance or physical EVT evidence defined by its authority record.

## Remaining development sequence

### B11 - ESP32-C6 and SDIO feasibility

1. Start from the committed B10 board.
2. Place U4 at a candidate board-owned outer edge with the module antenna facing outward.
3. Add and validate the ESP32-C6 antenna keepout on every copper layer.
4. Compact R44-R54 into the SDIO interface area with the clock series element at its source and the CMD/DAT pull-ups in the quiet `3V3_C6` area.
5. Route `C6_SDIO_CLK`, `C6_SDIO_CMD` and `C6_SDIO_D0..D3` between U1, the resistor network and U4.
6. Record route lengths, via counts, layer use, U4 orientation and keepout geometry in a B11 JSON authority and validator.
7. Require zero route-specific KiCad 10 DRC findings before the B11 checkpoint is committed.

### B12 - P4 power and return strategy

1. Complete the P4 power-pin escape and local decoupling connections.
2. Establish the continuous In1.Cu GND reference strategy and the first controlled GND stitching plan.
3. Build the In2.Cu power-distribution strategy without creating return-path splits under critical signals.
4. Review the QSPI, crystal, DCDC and SDIO return paths together.

### B13 - power spine and USB

1. Compact J1/U7/R120/U8/L2 and the 5 V to 3.3 V conversion area around a short high-current path.
2. Preserve INA238 Kelvin sensing at R120.
3. Place U6/U12 and route both independently switched USB VBUS branches.
4. Place J2/J3 and their ESD networks after the connector edge datums are available.
5. Route and validate USB0 HS and USB1 FS differential pairs against the final stackup calculator geometry.

### B14 - storage, display and audio

1. Place and route J7/U13 microSD power and four-bit SDMMC.
2. Place J6 at a service-accessible edge and route the generic two-lane MIPI DSI host interface.
3. Keep every display-specific cable and power condition in its compatibility profile.
4. Place U5 and the MAIN L/R output network in a quiet analog region near J4/J5.
5. Keep audio returns and traces away from switching nodes and high-current bottlenecks.

### B15 - board-owned mechanics and full routing

1. Derive the minimum viable PCB envelope from routed component packing.
2. Add chassis mounting holes and component-height zones from board/enclosure datums.
3. Resolve every connector cutout, mating and service envelope.
4. Complete all remaining signal and power routing.
5. Remove the eight remaining silkscreen findings.

### B16 - fabrication candidate

1. Record the exact JLCPCB 90 ohm USB and 100 ohm MIPI width/spacing calculator outputs.
2. Lock the production net-class rules and perform the final SI/return-path review.
3. Reach zero unexplained ERC and DRC violations and zero unconnected items.
4. Run schematic-to-board, BOM, DNP, footprint, polarity and pin-1 reviews.
5. Generate and inspect Gerber, drill, BOM and pick-and-place outputs only after every freeze blocker is closed.
6. Perform an independent fabrication-package review before authorizing the first EVT order.

## First action next session

Continue with B11. Inspect U4's current footprint geometry and antenna orientation, then create a reproducible placement/routing script from the committed B10 board. Do not begin from a manually altered or uncommitted canvas. Preserve all existing B10 critical routes and the project-local U8 footprint.

The B11 stop gate is a reviewable U4/SDIO candidate with an all-layer antenna keepout, a machine-readable report, a validator and a fresh KiCad 10 DRC result. If the candidate requires `Edge.Cuts`, final connector coordinates or enclosure assumptions, stop at the reversible placement study and leave those authorities open.

## Release boundary

Keep `layout_freeze_allowed=false`. Do not add production `Edge.Cuts`, release Gerbers, authorize an EVT order or describe the board as production-ready while any of the 12 blockers remains open.

Current detailed evidence:

- [Current Design Status B10](Pajoniiir_M1_Current_Design_Status_B10.md)
- [B10 P4 Critical Routes](Pajoniiir_M1_PRELAYOUT_B10_P4_Critical_Routes_v0.1.md)
- [Mechanical and Sourcing Gates](Pajoniiir_M1_Mechanical_Sourcing_Gates_v0.1.md)
- [PCB Placement and Routing Constraints](Pajoniiir_M1_PCB_Layout_Constraints_v0.1.md)
