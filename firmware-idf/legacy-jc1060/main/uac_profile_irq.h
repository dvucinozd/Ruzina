#pragma once
#include <stdbool.h>
#include <stdint.h>
/* Core-0 C-dispatch wall only. Nested sums are inclusive and invalidate union
 * attribution. Vector entry/exit, context switch and core 1 are not covered. */
typedef struct { uint32_t us, entries, nested, wall_us; bool valid; } uac_irq_stamp_t;
uac_irq_stamp_t ruzina_uac_irq_stamp(void);
void __wrap__global_interrupt_handler(intptr_t sp,int mcause);
