#pragma once
#include <stdbool.h>
#include <stdint.h>
typedef struct { uint32_t away_us, switches, wall_us; bool valid; } uac_sched_stamp_t;
bool ruzina_uac_sched_bind(const void *task);
uac_sched_stamp_t ruzina_uac_sched_stamp(void);
void ruzina_uac_switched_out(const void *task, unsigned core);
void ruzina_uac_switched_in(const void *task, unsigned core);
/* Last catalog event 1..6, published by SD task before its formatted log. */
void ruzina_uac_catalog_event(uint32_t event);
uint32_t ruzina_uac_catalog_phase(void);
