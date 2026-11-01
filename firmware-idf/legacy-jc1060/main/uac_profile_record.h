#pragma once
#include "uac_profile_sched.h"
enum { UAC_PROFILE_WORDS=20 };
typedef struct {
    uint32_t generation, ordinal, uptime_us, phase, pool_mask, health, catalog;
    uint32_t flags[4], log_drops, whole_us;
    uint32_t sched_window_us, sched_away_us, sched_switches, sched_valid;
    uint32_t rust[UAC_PROFILE_WORDS], rust_valid;
} uac_profile_record_t;
static inline void uac_profile_schedule(uac_profile_record_t *r,uac_sched_stamp_t before,uac_sched_stamp_t after) {
    r->sched_window_us=after.wall_us-before.wall_us;
    r->sched_away_us=after.away_us-before.away_us;
    r->sched_switches=after.switches-before.switches;
    r->sched_valid=before.valid && after.valid && r->sched_away_us<=r->sched_window_us;
}
