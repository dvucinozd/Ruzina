#pragma once
#include <stdbool.h>
#include <stdint.h>

/* Client-task-owned, boot-cumulative counters. Counts saturate, never wrap.
 * The caller supplies the phase: health state alone cannot distinguish prefill.
 * Durations are whole-fill wall time, including preemption, not CPU-only time. */
enum uac_fill_phase { UAC_FILL_INITIAL_PREFILL, UAC_FILL_RECOVERY_PREFILL,
                      UAC_FILL_CALLBACK, UAC_FILL_PHASE_COUNT };
typedef struct {
    uint32_t count, first_us, max_us, gt_3000, ge_6000, failures;
    bool first_after_iso_submit, max_after_iso_submit;
} uac_fill_phase_stats_t;
typedef struct {
    uac_fill_phase_stats_t phase[UAC_FILL_PHASE_COUNT];
    uint32_t first_connection_us;
    enum uac_fill_phase first_connection_phase;
    bool first_connection_seen, first_connection_after_iso_submit;
} uac_fill_timing_t;

static inline void uac_timing_increment(uint32_t *value) {
    if (*value != UINT32_MAX) ++*value;
}
static inline void uac_timing_connection_begin(uac_fill_timing_t *s) {
    s->first_connection_seen=false;
    s->first_connection_us=0;
    s->first_connection_phase=UAC_FILL_INITIAL_PREFILL;
    s->first_connection_after_iso_submit=false;
}
static inline void uac_timing_record(uac_fill_timing_t *s, enum uac_fill_phase phase,
                                     uint32_t us, bool after_iso_submit, bool failed) {
    uac_fill_phase_stats_t *p=&s->phase[phase];
    if (!p->count) { p->first_us=us; p->first_after_iso_submit=after_iso_submit; }
    if (!p->count || us > p->max_us) { p->max_us=us; p->max_after_iso_submit=after_iso_submit; }
    uac_timing_increment(&p->count);
    if (us > 3000) uac_timing_increment(&p->gt_3000);
    if (us >= 6000) uac_timing_increment(&p->ge_6000);
    if (failed) uac_timing_increment(&p->failures);
    if (!s->first_connection_seen) {
        s->first_connection_seen=true;
        s->first_connection_us=us;
        s->first_connection_phase=phase;
        s->first_connection_after_iso_submit=after_iso_submit;
    }
}
