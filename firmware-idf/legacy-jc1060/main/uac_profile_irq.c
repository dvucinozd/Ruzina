#include <stdatomic.h>
#include "uac_profile_irq.h"
#ifdef UAC_PROFILE_TEST
#define IRAM_ATTR
extern int64_t esp_timer_get_time(void);
extern unsigned uac_test_core(void);
#define PROFILE_CORE() uac_test_core()
#else
#include "esp_attr.h"
#include "esp_timer.h"
#include "riscv/rv_utils.h"
#define PROFILE_CORE() rv_utils_get_core_id()
#endif
_Static_assert(ATOMIC_INT_LOCK_FREE==2,"IRQ diagnostic requires lock-free atomics");
static _Atomic uint32_t epoch,depth,total_us,entries,nested;
static _Atomic uint32_t enabled;
extern void __real__global_interrupt_handler(intptr_t sp,int mcause);
void IRAM_ATTR __wrap__global_interrupt_handler(intptr_t sp,int mcause) {
    if(PROFILE_CORE()!=0 || !atomic_load_explicit(&enabled,memory_order_relaxed)) {
        __real__global_interrupt_handler(sp,mcause); return;
    }
    /* Mark the entire timing bracket active. Higher-priority IRQs may nest;
     * count them explicitly, never pretend their inclusive sum is a union. */
    uint32_t prior=atomic_fetch_add_explicit(&depth,1,memory_order_seq_cst);
    atomic_fetch_add_explicit(&epoch,1,memory_order_seq_cst);
    atomic_fetch_add_explicit(&entries,1,memory_order_relaxed);
    if(prior) atomic_fetch_add_explicit(&nested,1,memory_order_relaxed);
    uint32_t start=(uint32_t)esp_timer_get_time();
    __real__global_interrupt_handler(sp,mcause);
    uint32_t elapsed=(uint32_t)esp_timer_get_time()-start;
    atomic_fetch_add_explicit(&total_us,elapsed,memory_order_relaxed);
    atomic_fetch_add_explicit(&epoch,1,memory_order_seq_cst);
    atomic_fetch_sub_explicit(&depth,1,memory_order_seq_cst);
}
uac_irq_stamp_t ruzina_uac_irq_stamp(void) {
    /* Do not read the systimer from early-boot interrupts before the first
     * USB-task snapshot. Coverage starts here, after timer initialization. */
    atomic_store_explicit(&enabled,true,memory_order_seq_cst);
    uint32_t before=atomic_load_explicit(&epoch,memory_order_seq_cst);
    bool idle=atomic_load_explicit(&depth,memory_order_seq_cst)==0;
    uac_irq_stamp_t s={.us=atomic_load_explicit(&total_us,memory_order_relaxed),
        .entries=atomic_load_explicit(&entries,memory_order_relaxed),
        .nested=atomic_load_explicit(&nested,memory_order_relaxed),
        .wall_us=(uint32_t)esp_timer_get_time()};
    atomic_thread_fence(memory_order_seq_cst);
    s.valid=PROFILE_CORE()==0 && idle && atomic_load_explicit(&depth,memory_order_seq_cst)==0
        && before==atomic_load_explicit(&epoch,memory_order_seq_cst);
    return s;
}
