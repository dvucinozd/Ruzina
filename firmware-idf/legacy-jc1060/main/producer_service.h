#pragma once
#include <stdbool.h>
#include <stdint.h>
#include <limits.h>

/* Client wall times include blocked handle_events and callback execution.
 * Callback-entry spacing is an observation, not a hardware IRQ timestamp. */
typedef struct { uint32_t count,max_us; uint64_t total_us; } producer_service_metric_t;
typedef struct {
    uint32_t generation,invalid,saturated;
    uint64_t last_callback_us;
    bool callback_seen;
    producer_service_metric_t handle,tail,callback_gap;
} producer_service_t;

static inline void producer_service_add(producer_service_t *s,producer_service_metric_t *m,uint64_t us) {
    if(m->count==UINT32_MAX || UINT64_MAX-m->total_us<us || us>UINT32_MAX) s->saturated=1;
    if(m->count<UINT32_MAX) ++m->count;
    m->total_us=UINT64_MAX-m->total_us<us ? UINT64_MAX : m->total_us+us;
    uint32_t bounded=us>UINT32_MAX ? UINT32_MAX : (uint32_t)us;
    if(bounded>m->max_us) m->max_us=bounded;
}
static inline void producer_service_loop(producer_service_t *s,uint32_t generation,bool active,
                                         uint64_t start,uint64_t handled,uint64_t end) {
    if(!active || generation!=s->generation) return;
    if(start>handled || handled>end) { if(s->invalid<UINT32_MAX) ++s->invalid;return; }
    producer_service_add(s,&s->handle,handled-start);
    producer_service_add(s,&s->tail,end-handled);
}
static inline void producer_service_callback(producer_service_t *s,uint64_t now) {
    if(s->callback_seen) {
        if(now<s->last_callback_us) { if(s->invalid<UINT32_MAX) ++s->invalid; }
        else producer_service_add(s,&s->callback_gap,now-s->last_callback_us);
    }
    s->last_callback_us=now;s->callback_seen=true;
}

#define PRODUCER_PROFILE_WORDS 40
static inline uint64_t producer_profile_u64(const uint32_t *words) {
    return (uint64_t)words[0] | ((uint64_t)words[1]<<32);
}
