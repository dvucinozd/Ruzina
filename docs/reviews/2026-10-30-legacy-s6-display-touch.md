# S6 JC1060 New_Panel display/touch diagnostic

Date: 2026-10-30. Unit JC1060-20261030-COM3 / P4 v1.3. Scope is an isolated
display/touch diagnostic after the [S5 memory/task proof](2026-10-30-legacy-s5-memory-task.md).
SD, USB host, audio and C6 remain inactive. Product readiness remains NOT_QUALIFIED.

## Source and identity review

The owner reports New_Panel; PCB/module/panel markings remain unavailable
without case removal. The V1.0 schematic PDF was rendered and visually reviewed,
including enlarged module/pin-map and touch sections. The supplied specification
and getting-started PDFs were extracted for context. Document hashes and exact
source/timing choices are in the
[source review manifest](../boards/readiness/jc1060-s06-display-source-review-20261030.json).

The schematic connects GPIO0 to LCD reset, GPIO23 to LCD_PWM, GPIO7/8 to the
shared SDA/SCL bus and GPIO21/22 to TOUCH_INT/TOUCH_RST. The New_Panel BSP uses
the first four pins but leaves touch reset/interrupt NC. This is a software
ownership choice, not proof that those wires are absent. The diagnostic does
not configure GPIO21/22; it polls the existing GT911 address and leaves its
reset/address straps unchanged. I2C100 kHz is limited to GT9110x5d/0x14; no bus
scan or codec/C6 configuration is performed.

The factory backup contains a60-record table at flash 0x73fd0, stride 16, matching
every New_Panel command/length/delay.48 payloads also compare directly via the
factory app segment address map.12 zero-valued payload pointers target RAM
outside the initialized segments, consistent with BSS but not directly provable
from the dump. The 56-record Old_Panel metadata table has no matching candidate.
This supports New_Panel source selection alongside the owner report and prior
visible factory demo; it does not read a physical controller marking.
Both vendor variants' stale EK79007 log messages remain unusable as identity.

The 60 records were adapted to fixed-size data arrays in
`firmware-idf/legacy-jc1060/main/new_panel_commands.h`. Source Apache-2.0
copyright/license are preserved with a provenance NOTICE and adjacent LICENSE
copy. The canonical record hash test locks command/data/length/delay order.
No wildcard registry dependency or whole vendor BSP was imported. DSI/DPI,
I2C, LEDC, LDO and cache operations use the pinned IDF 6.1 source directly.

## Diagnostic ownership and lifecycle

The separate `display-touch` profile uses PSRAM 80 MHz capability allocation,
360 MHz CPU and no XiP; it cannot coexist with the memory-task owner. Scalar and
pure-RAM profiles still reject PSRAM. Configuration tests cover ownership,
frequency and pure-RAM exclusions. A single driver-owned RGB565 framebuffer
of 1,228,800 bytes is filled before `esp_lcd_panel_init` starts scanout and is
cache-written back through the IDF API. The static pattern is never modified
during scanout; this does not qualify later animated/double-buffer rendering.

Backlight starts off on GPIO23/LEDC20 kHz,10-bit. LDO3 supplies the DPHY at 2500mV;
DSI uses 2 lanes at 750Mbps. Pixel clock 52 MHz,1024x600, horizontal back/pulse/front
136/24/160 and vertical 21/2/12 follow the actual New_Panel implementation,
not conflicting stale header macros. The table exits sleep with 120 ms delay and
enables display with 20 ms delay. After successful init the backlight is 25%.

The static screen has eight left-to-right stripes: red, green, blue, white,
black, yellow, cyan, magenta, with white 32x32 corner markers. Operator evidence
must confirm order, stable image, complete borders and corners. No display
controller ID claim is inferred from a successful command transmission.

GT911 product ID must begin 911. Current RAM configuration supplies x/y extent;
values must be 2..4096. Coordinates map to 0..1023 and 0..599 with clamping; no
assumption of 800x480 is made. Polling reads ready points and writes only the
ready-status acknowledgment, never persistent configuration. Five-point count
is bounded; repeated health lines report samples/errors. Physical orientation,
corner alignment and multi-touch are not accepted from ID reads alone.

On initialization error, backlight turns off and handles remain until a
controlled app restart; no concurrent touch task exists on those failure paths.
Runtime teardown/reallocation and arbitrary repeated driver init are not yet
implemented. App reset/cold boot must separately demonstrate startup stability.
No new product framebuffer allocator or UI renderer is activated.

## Validation and execution

Twelve offline test methods pass, including the exact 60-record table hash.
Fresh `s06-reviewed-display-touch` build returned BUILD_PACKAGE_PASS, including
Rust fmt/Clippy, linked ABI/header/map and source/artifact hash checks.
App218528 bytes, SHA-256
`2d6686e2b3a95b6a91b27d0c069d6b270c040e5d1c485b15d6b9e355caa22f7d`.
The first development builds exposed an undeclared esp_mm/cache dependency and
the framebuffer API's void-pointer out-parameter; both were corrected without
suppressing compiler diagnostics before the accepted fresh package.
Fresh scalar regression also passed, without display initialization enabled.
The S5 predecessor plan supplies expected live sectors and rollback content;
new writes require matching live identity/security and predecessor-sector hashes.
The S5 evidence archive remains immutable.

The reviewed package was written after live identity/security and predecessor
sector comparisons; every affected sector was read back and hash-verified:
[programming result](../boards/evidence/jc1060-20261030-com3/s06-first-flash-result.json),
[recovery plan](../boards/readiness/jc1060-s06-write-recovery-plan-20261030.json).
The external writer was adapted from the scalar script and its result retains
the stale descriptive string "no BSP peripheral access"; that describes the
programming stage only and must not describe the later display/touch execution.
The actual S6 pin/resource scope is documented above.

First boot observed scalar match, S6_DISPLAY_INIT, S6_TOUCH_ID and 43 heartbeats,
with no init failure or HP watchdog:
[boot result](../boards/evidence/jc1060-20261030-com3/s06-first-boot-result.json).
GT911 ID39313100 at 0x5d reports current RAM config 1024x600. The stale factory
800x480 calibration log is therefore not used. Initial polling reported zero
I2C errors, with no operator touches during that window.

The owner confirmed the expected stripe order and all four visible corners.
Three app restarts each reinitialized display/GT911, observed scalar match and
continued 10–11 heartbeats without init failure or HP watchdog:
[restart results](../boards/evidence/jc1060-20261030-com3/s06-app-restart-result.json).
This tests reset/startup lifecycle, not in-process resource deallocation/reinit.

The first operator touch campaign recorded 65 samples across TL/TR/BL/BR/center,
reported count 2, zero health errors and all mapped points in range:
[touch result](../boards/evidence/jc1060-20261030-com3/s06-touch-corners-result.json).
The bottom zones arrived BL then BR rather than the requested BR then BL, so
a separate named-order verification is required before orientation acceptance.
The initial clusters were TL11,7; TR1004,23; BL19,581; BR1006,589; center 477,276.
The logger prints only the first point, so count 2 proves simultaneous detection,
not independent coordinate/ID tracking acceptance for both points.

The owner removed the sole USB power for 5–10 seconds and reconnected without
BOOT/RESET, then confirmed the same correct stripes returned. The
[cold capture](../boards/evidence/jc1060-20261030-com3/s06-cold-boot-result.json)
includes app reset_reason1, scalar match, display init and GT911 identity.
Its earlier heartbeat count includes pre-disconnect runtime; do not interpret
that count as 26 post-cold heartbeats. The earliest ROM boot prefix is missing.
This is one operator-correlated cold startup, not a three-cold-cycle or long-soak
qualification. Subsequent corner capture verifies continuing post-cold runtime.

The owner then explicitly touched BL followed by BR in the same screen
orientation. The [second corner capture](../boards/evidence/jc1060-20261030-com3/s06-bottom-order-result.json)
records 40 samples, ordered BL16,578 then BR1010,578, with no health errors.
This closes the initial bottom-order ambiguity for the bounded orientation
smoke; it does not imply subpixel calibration or exhaustive edge accuracy.

## Handoff and remaining gates

The board is left running the static display/polling-touch diagnostic at 25%
backlight. The owner accepted stripe order/corners and their return after cold
power removal. Startup/ID logs, point samples, three app restarts and one cold
startup are retained for this exact image. The source review,12 offline tests,
target fmt/Clippy, fresh display and scalar builds passed. No full product suite,
remote CI, commit or push was performed. Native minimum 302 and shared product
app/crate sources remain unchanged. PCB/module/panel markings remain unavailable.

Remaining display/touch gates: in-process clean stop/reinit, framebuffer swap/
animated cache/DMA behavior, physical rotation scenarios, brightness range,
independent multi-touch ID/coordinate tracking, long soak and product UI adapter.
Polling deliberately avoids unqualified GPIO21/22 control; interrupt/reset
operation remains a separate test. Static framebuffer success must not promote
all PSRAM/DMA paths to PASS.

Next planned step: prepare read-only SD ownership/power/source review, then
mount/read/removal tests with known disposable media, retaining generation/
cancellation boundaries. Do not activate SD writes, USB host/VBUS, audio or C6
as a side effect. Factory restore remains NOT_RUN; S5 rollback and original
factory backup remain available outside Git.

Final preservation: [S6 evidence package](../boards/readiness/jc1060-s06-evidence-package-20261030.json)
records a5,248,609-byte ZIP with 91 entries. ZIP CRC, every member SHA-256,
and the secondary BoardBackups copy were verified. ZIP SHA-256:
`07d792d97932403ce4d3b8b87f35b6fbe861e3815e9a3de07360d7721baa0e0c`.
It contains source/artifact/runtime/source-review snapshots, no factory dump.
Final checks: 12 offline methods PASS, unit validator PASS,180 documentation
links PASS before this final package link, diff whitespace PASS. Current
changes remain local/uncommitted and remote CI has not run.

Reference: [Espressif JD9165 component](https://components.espressif.com/components/espressif/esp_lcd_jd9165/versions/2.0.2/readme)
was consulted to confirm the controller/interface ecosystem, but no driver from
that registry page was imported. Local vendor pin/table records and the pinned
IDF API source govern this narrow experiment.
