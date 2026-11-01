# JC1060 external MAIN and headphone audio: future development

Recorded: 2026-11-01, at the owner's request.
Status: **PLANNED / SCHEMATIC_FEASIBILITY_ONLY / NOT_WIRED / NOT_TESTED**.

This is a continuation task, not an instruction to drive pins or replace the
current firmware. It applies to the observed JC1060P470C_I_W_Y v1.3 unit only.
The physical PCB marking has not been confirmed against the supplied V1.0
schematic. No evidence is transferred to JC8012 or native v3.2 boards. Keep
the native minimum revision **302** unchanged.

## Intended modules and behavior

| Function | Proposed module | Control address |
|---|---|---|
| Stereo MAIN | Waveshare PCM5122 Audio Board (A), with RCA L/R outputs | `0x4C` |
| Stereo headphones / CUE / PFL | TLV320DAC3100 breakout, as on Adafruit 6309 | `0x18` |

These are proposed development modules, not physically qualified or confirmed
as purchased. The owner's headphones are **Audio-Technica ATH-M20x**, specified
at 47 ohms and 96 dB/mW. Actual headphone output level, clipping, noise and
channel separation still need measurement/listening acceptance on the module.
Keep the TLV320 speaker amplifier disabled when using its headphone output.

Two independent stereo mixes are required. The PCM5122 board's RCA and 3.5 mm
connectors do not establish a second independent CUE/PFL mix. Start with two
I2S TX controllers, each carrying its own stereo mix; shared clocks are an
optional later optimization, not a condition of this pin budget.

Planned Settings behavior:

| Setting | Choices and semantics |
|---|---|
| MAIN output | `Controller`, `External DAC`, `Auto`. Auto prefers a configured, streaming DAC, then compatible controller MAIN, then mute. |
| Headphone output | `Controller` or `External headphone DAC`, independently of MAIN. Preserve Controller as the existing default; do not invent an automatic headphone fallback. |

An unavailable explicitly selected output must be shown as unavailable and
muted on its own lane. Use bounded fade/drain/rebind/fade-in transitions for
planned changes, immediate revocation on loss, and no credit for stale DMA/UAC
completions. A MAIN change must not alter the headphone mix, level or route.
I2C ACK alone is not readiness: configure/read back the device and establish
the stream before publishing it as usable.

Settings select the destination of digital audio. Headphones plug into the
selected device's jack. One common jack switching between controller analog
output and external analog output would require a separately designed analog
switching circuit; it is outside this two-module wiring plan.

## Important correction: separate external I2C bus

Do **not** simply attach the TLV320DAC3100 to the existing GPIO7/8 bus. The
vendor JC1060 demo configures the onboard **ES8311 at `0x18`**, colliding with
the TLV320DAC3100. Leaving ES8311 unused in software does not remove a powered
device from the bus.

Keep existing I2C0 on GPIO7/8 for touch/onboard devices. Put both external DACs
on **I2C1**, with candidate SDA45/SCL46. The two external addresses (`0x4C` and
`0x18`) are distinct on that separate bus. Check actual addresses, pullups to
3.3 V, bus ownership and rise times before activation. Any HAT EEPROM wiring
must also be checked; Raspberry Pi's automatic driver-loading mechanism is
not part of our firmware.

## Candidate JP1 pin assignment

Source: the `Expand IO` block of the supplied JC1060 V1.0 electrical schematic,
not the mechanical structure drawing. The ten candidate GPIOs on JP1 are
`1, 2, 3, 4, 5, 32, 33, 45, 46, 47`. GPIO20 is not counted as free because the
schematic assigns it to touch interrupt. C6 service/control pins are not free
P4 GPIOs and are excluded.

| Signal | P4 GPIO | JP1 physical pin |
|---|---:|---:|
| PCM5122 MAIN BCLK | 1 | 7 |
| PCM5122 MAIN LRCK / WS | 2 | 9 |
| PCM5122 MAIN DATA to DAC DIN | 3 | 11 |
| TLV320 headphone BCLK | 4 | 13 |
| TLV320 headphone LRCK / WS | 5 | 15 |
| TLV320 headphone DATA to DAC DIN | 32 | 19 |
| External I2C SDA | 45 | 14 |
| External I2C SCL | 46 | 12 |
| TLV320 RESET | 33 | 21 |
| Unassigned reserve | 47 | 10 |

This uses **9 of 10 candidate GPIOs**: six audio signals, two external I2C
signals and one reset. Both DACs must be configured to derive their clock from
BCLK so no separate MCLK pin is required. Optional interrupt, jack-detect or
hardware-mute lines are not allocated by this plan.

The candidate map avoids display GPIO0/23, existing I2C GPIO7/8, SD GPIO39..44,
onboard codec audio pins and the known C6/Ethernet/USB nets. This is a source
audit, not continuity or runtime proof. GPIO numbers and JP1 physical pin
numbers are different namespaces; confirm connector orientation and populated
hardware before wiring. The Waveshare 40-pin Raspberry Pi header cannot be
plugged directly into JP1; use a verified adapter or wiring harness.

Power is a separate gate. Waveshare specifies 5 V supply and 3.3 V logic for
its board. Verify the actual supply rail and current budget with USB/controller
and display active. JP1 pins 2/4 are labelled `VOUT-BAT`; do not treat that
label as proof of a suitable regulated 5 V source. Confirm common ground and
module regulator/pullup wiring before connecting either module.

## Implementation sequence for the next development batch

1. Re-read the latest handoff, physical unit identity, source schematic and
   active pin owners. Confirm JP1 numbering, continuity, supply voltage/current,
   exact module variants and the absence of conflicts on the physical board.
2. Introduce the optional external-headphone requirement and route/settings
   tests deliberately. Current controller-only requirements and tests describe
   the existing implementation; this plan is its future extension, not an
   already enabled alternate route. Preserve controller defaults and distinct
   MAIN/PFL mixes. Update architecture/requirements when implementing it.
3. Add a disabled-by-default JC1060 external-audio profile and board pin contract.
   Keep I2C0/touch ownership intact and give I2C1 one external-audio control
   owner. Test address collision prevention and initialization failures offline.
4. Implement PCM5122 control, I2S TX and safe mute/start/stop for MAIN first.
   Verify register configuration, BCLK/LRCK/sample rate, channel polarity,
   amplitude and fault/loss behavior before integrating product playback.
5. Add TLV320 reset/PLL/headphone-driver sequencing and a second independent
   stereo stream. Verify ATH-M20x levels, silence, clipping, separation and
   power-cycle behavior. Disable the unused speaker amplifier.
6. Add persistent Settings and independent output owners. Cover unavailable
   manual selections, Auto MAIN fallback, stale completions, controller detach,
   DAC loss/recovery and bounded route changes while preserving the other lane.
7. Run simultaneous MAIN/PFL with display, touch, SD reads and controller UAC/MIDI.
   Measure DMA/CPU/PSRAM load, underruns, clock accuracy/drift and transition
   noise. Qualify the exact flashed image and record operator observations.

Pinned IDF 6.1 exposes three I2S instances and two HP I2C controllers. Its P4
HAL selects XTAL as the default I2S source below revision 3, rather than the
v3 PLL default. Do not copy a v3 clock assumption into this v1.3 profile; two
working streams and their timing remain target qualification gates.

## Provenance and remaining acceptance gates

Local source package:
`C:/Users/Daniel/Desktop/ESP-BOARDS/JC1060P470C_I_W_Y`.

- Electrical schematic: `5-Schematic/JC1060P470C_I_W_Y-V1.0.pdf`, SHA-256
  `ae3f41fe9a66b201358ef8cf1f531fdced68c63f84c8bde36e23cde5132a23ee`.
- Existing ES8311 address/pins: `1-Demo/Demo_Arduino/JC1060P470C_I_W_Y_New_Panel/mp3_player/mp3_player.ino`, SHA-256
  `918901455e4409288cd03a8ba63072d6822a701f7e6cd1cc3127423e2564ea16`.
- Initial pin audit checked main `bd52402160af27a3e234f7647d703a847644817b`.
  Re-check current owners before implementation; development continues elsewhere.
- Local review renders and candidate JSON:
  `D:/BoardEvidence/JC1060-20261030-COM3/pin-audit-20261031`.

Primary references:
[Waveshare module](https://www.waveshare.com/pcm5122-audio-board-a.htm),
[Waveshare pin/power documentation](https://www.waveshare.com/wiki/PCM5122-Audio-Board-A),
[TLV320 module](https://www.adafruit.com/product/6309),
[TLV320 pinout](https://learn.adafruit.com/adafruit-tlv320dac3100-i2s-dac/pinouts),
[PCM5122](https://www.ti.com/product/PCM5122),
[TLV320DAC3100](https://www.ti.com/product/TLV320DAC3100),
[ATH-M20x specifications](https://www.audio-technica.co.jp/pdf/support/ATH-M20x_UM.pdf),
[IDF 6.1 I2S](https://docs.espressif.com/projects/esp-idf/en/v6.1/esp32p4/api-reference/peripherals/i2s.html).

Open: physical board/connector match, harness, power budget, drivers/settings,
two-output clocking, level/listening, transitions, reconnect/cold boot and
combined-load qualification. No COM3 access, wiring, flash or physical DAC test
was performed for this planning record. Historical bring-up evidence is unchanged.
