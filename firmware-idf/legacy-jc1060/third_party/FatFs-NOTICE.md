# Private read-only FatFs instance

The SD diagnostic builds pristine `ff.c`, `ff.h`, `ffunicode.c` and `diskio.h`
from ESP-IDF commit fff9895c82d744c7237be8847347bdd1b07c6643. These files retain
their original ChaN/FatFs R0.16 copyright/redistribution notice. They are copied
only into the diagnostic build directory. The global IDF checkout is unchanged.
Only the separate `main/fatfs_ro/ffconf.h` is project configuration: read-only,
exFAT, LFN UTF-8, one single-task-owned volume, no format/trim/write APIs.
The build manifest records the four pristine source hashes. No generic IDF
FatFs VFS/read-write component is linked. These diagnostic settings do not
change the shared Rust FAT/exFAT implementation or select a product FS owner.
