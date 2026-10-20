# m1-factory

Reserved for a complete factory/bring-up firmware. The initial Guition service-only report is implemented in `apps/m1-firmware` behind an explicit board selector plus `board-diagnostics`; see [first-boot preparation](../../../docs/boards/GUITION_FIRST_BOOT_2026-10-20.md).

The historical custom-board checks below still require a per-Guition disposition; they are not claims that those components are fitted:

- P4 revision >= product minimum;
- 16 MiB external flash and 32 MiB PSRAM;
- power rails and INA238;
- USB0/USB1 VBUS control and fault inputs;
- USB HS/FS bring-up;
- microSD;
- C6 SDIO/Wi-Fi;
- DSI/touch;
- PCM5102A test tone;
- UART0 and USB Serial/JTAG service paths.

Hardware checks remain pending for the arriving Guition units. Custom PCB EVT fabrication is no longer a prerequisite. The current diagnostic image reads chip revision/reset reason; memory identity, recovery and all peripheral tests remain unqualified.
