# m1-factory

Reserved for the M1 factory/bring-up firmware.

Planned checks include:

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

The target remains HARDWARE_PENDING until EVT hardware exists.
