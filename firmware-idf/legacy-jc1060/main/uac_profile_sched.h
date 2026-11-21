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
/* What else ran while a fill did: the screen drawing a frame, the decoders on the other
 * core, and the media owner reading the stick. Each counts its edges and is odd while it
 * runs; only its own task marks it. The screen also says when it stands still on purpose
 * (`--screen-still`). */
enum { UAC_ACTIVITY_SCREEN, UAC_ACTIVITY_DECODER, UAC_ACTIVITY_STICK, UAC_ACTIVITIES };
typedef struct { uint32_t edges[UAC_ACTIVITIES]; bool still; } uac_activity_stamp_t;
void ruzina_uac_activity(uint32_t which, bool running);
void ruzina_uac_screen_still(bool still);
uac_activity_stamp_t ruzina_uac_activity_stamp(void);
