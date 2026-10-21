# Factory and service scope

Guition unit/service qualification currently uses the main firmware's explicit
diagnostic variants and package/capture tool. This directory does not contain an
independent factory executable.

Record each model/unit and factory image, verify backup and ROM recovery, then
qualify boot/service, flash/PSRAM, display/touch, storage, USB power/data and audio
individually. No peripheral or factory acceptance is inferred from build results.

See [first boot](../../../docs/boards/GUITION_FIRST_BOOT_2026-10-20.md) and
[development status](../../../docs/DEVELOPMENT.md).
