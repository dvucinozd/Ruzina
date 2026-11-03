#include <stdatomic.h>
#include <stddef.h>
#include "uac_profile_sched.h"
#ifdef UAC_PROFILE_TEST
#define IRAM_ATTR
extern int64_t esp_timer_get_time(void);
#else
#include "esp_attr.h"
#include "esp_timer.h"
#endif
_Static_assert(ATOMIC_INT_LOCK_FREE==2 && ATOMIC_POINTER_LOCK_FREE==2,"Diagnostic hooks require lock-free 32-bit atomics");
static _Atomic(const void *) tracked;
static _Atomic uint32_t sequence, away, switches, out_since, is_out, catalog;

bool ruzina_uac_sched_bind(const void *task) {
    const void *expected=NULL;
    if (!task) return false;
    return atomic_compare_exchange_strong(&tracked,&expected,task) || expected==task;
}
void IRAM_ATTR ruzina_uac_switched_out(const void *task,unsigned core) {
    if (core!=0 || !task || task!=atomic_load_explicit(&tracked,memory_order_relaxed)) return;
    atomic_fetch_add_explicit(&sequence,1,memory_order_acq_rel);
    atomic_store_explicit(&out_since,(uint32_t)esp_timer_get_time(),memory_order_relaxed);
    atomic_store_explicit(&is_out,1,memory_order_relaxed);
    atomic_fetch_add_explicit(&sequence,1,memory_order_release);
}
void IRAM_ATTR ruzina_uac_switched_in(const void *task,unsigned core) {
    if (core!=0 || !task || task!=atomic_load_explicit(&tracked,memory_order_relaxed)) return;
    atomic_fetch_add_explicit(&sequence,1,memory_order_acq_rel);
    if (atomic_load_explicit(&is_out,memory_order_relaxed)) {
        uint32_t elapsed=(uint32_t)esp_timer_get_time()-atomic_load_explicit(&out_since,memory_order_relaxed);
        atomic_fetch_add_explicit(&away,elapsed,memory_order_relaxed);
        atomic_fetch_add_explicit(&switches,1,memory_order_relaxed);
        atomic_store_explicit(&is_out,0,memory_order_relaxed);
    }
    atomic_fetch_add_explicit(&sequence,1,memory_order_release);
}
uac_sched_stamp_t ruzina_uac_sched_stamp(void) {
    /* One bounded attempt; interrupt/preemption during sampling makes it invalid.
     * Atomic words avoid data races/tearing on RV32. No scheduler masking. */
    uint32_t before=atomic_load_explicit(&sequence,memory_order_acquire);
    uac_sched_stamp_t stamp={.away_us=atomic_load_explicit(&away,memory_order_relaxed),
        .switches=atomic_load_explicit(&switches,memory_order_relaxed),.wall_us=(uint32_t)esp_timer_get_time()};
    bool running=!atomic_load_explicit(&is_out,memory_order_relaxed);
    atomic_thread_fence(memory_order_acquire);
    stamp.valid=!(before&1) && running && atomic_load_explicit(&tracked,memory_order_relaxed)!=NULL
        && before==atomic_load_explicit(&sequence,memory_order_acquire);
    return stamp;
}
void ruzina_uac_catalog_event(uint32_t event) {
    if (event>=1 && event<=6) atomic_store_explicit(&catalog,event,memory_order_release);
}
uint32_t ruzina_uac_catalog_phase(void) { return atomic_load_explicit(&catalog,memory_order_acquire); }
