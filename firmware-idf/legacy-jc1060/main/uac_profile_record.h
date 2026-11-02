#pragma once
#include "uac_profile_sched.h"
#include "uac_profile_irq.h"
enum { UAC_PROFILE_WORDS=20 };
typedef struct {
    uint32_t generation, ordinal, uptime_us, phase, pool_mask, health, catalog;
    uint32_t flags[4], log_drops, whole_us;
    uint32_t sched_window_us, sched_away_us, sched_switches, sched_valid;
    uint32_t sched_away_before, sched_away_after, sched_switches_before, sched_switches_after;
    uint32_t calibration_passed, irq_calibration_passed;
    uint32_t irq_window_us, irq_us, irq_entries, irq_nested, irq_valid;
    uint32_t rust[UAC_PROFILE_WORDS], rust_valid;
} uac_profile_record_t;
static inline void uac_profile_schedule(uac_profile_record_t *r,uac_sched_stamp_t before,uac_sched_stamp_t after) {
    r->sched_window_us=after.wall_us-before.wall_us;
    r->sched_away_us=after.away_us-before.away_us;
    r->sched_switches=after.switches-before.switches;
    r->sched_away_before=before.away_us; r->sched_away_after=after.away_us;
    r->sched_switches_before=before.switches; r->sched_switches_after=after.switches;
    r->sched_valid=before.valid && after.valid && r->sched_window_us>=r->whole_us && r->sched_away_us<=r->sched_window_us;
}
static inline bool uac_profile_calibration(uac_sched_stamp_t before,uac_sched_stamp_t after) {
    return before.valid && after.valid && after.switches-before.switches>0
        && after.away_us-before.away_us>0 && after.away_us-before.away_us<=after.wall_us-before.wall_us;
}
static inline void uac_profile_interrupts(uac_profile_record_t *r,uac_irq_stamp_t before,uac_irq_stamp_t after) {
    r->irq_window_us=after.wall_us-before.wall_us;
    r->irq_us=after.us-before.us; r->irq_entries=after.entries-before.entries;
    r->irq_nested=after.nested-before.nested;
    /* Inclusive nested dispatch times must never be subtracted as a union. */
    r->irq_valid=before.valid && after.valid && !r->irq_nested && r->irq_window_us>=r->whole_us && r->irq_us<=r->irq_window_us;
}
static inline bool uac_profile_partition(const uac_profile_record_t *r) {
    uint64_t sum=0; for(unsigned i=0;i<8;++i) sum+=r->rust[4+i];
    return r->rust_valid && !r->rust[1] && sum==r->rust[0] && r->rust[0]<=r->whole_us;
}
enum { UAC_PROFILE_BINS=7 };
typedef struct { uint32_t samples, valid, whole[UAC_PROFILE_BINS], stage[8][UAC_PROFILE_BINS], max[8]; } uac_profile_distribution_t;
static inline void uac_profile_increment(uint32_t *v) { if(*v!=UINT32_MAX) ++*v; }
static inline unsigned uac_profile_bin(uint32_t us) {
    const uint32_t upper[]={50,100,250,500,1000,3000};
    for(unsigned i=0;i<6;++i) if(us<=upper[i]) return i;
    return 6;
}
static inline void uac_profile_distribute(uac_profile_distribution_t *d,const uac_profile_record_t *r) {
    uac_profile_increment(&d->samples);
    uac_profile_increment(&d->whole[uac_profile_bin(r->whole_us)]);
    if(!uac_profile_partition(r)) return;
    uac_profile_increment(&d->valid);
    for(unsigned i=0;i<8;++i) {
        uint32_t us=r->rust[4+i]; uac_profile_increment(&d->stage[i][uac_profile_bin(us)]);
        if(us>d->max[i]) d->max[i]=us;
    }
}
