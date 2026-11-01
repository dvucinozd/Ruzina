#pragma once
#include <stdbool.h>
#include <stdint.h>

/* Client-task-only recovery policy; endpoint/DMA ownership stays in the driver.
 * Three recoveries per connection, deliberately conservative until measured. */
enum uac_health { UAC_HEALTH_OFF, UAC_HEALTH_STARTING, UAC_HEALTH_STREAMING,
                  UAC_HEALTH_RECOVERING, UAC_HEALTH_STOPPING, UAC_HEALTH_FAULT_HELD };
typedef struct {
    enum uac_health state;
    uint32_t generation;
    unsigned attempts;
    bool flushed;
    unsigned wait_turns;
} uac_recovery_t;
enum { UAC_RECOVERY_LIMIT = 3, UAC_DRAIN_TURNS = 50 };
static inline void uac_recovery_begin(uac_recovery_t *s, uint32_t generation) {
    *s = (uac_recovery_t){.state=UAC_HEALTH_STARTING, .generation=generation};
}
static inline bool uac_recovery_current(const uac_recovery_t *s, uint32_t generation) {
    return s->generation == generation;
}
static inline void uac_recovery_fault(uac_recovery_t *s) {
    if (s->state == UAC_HEALTH_STREAMING || s->state == UAC_HEALTH_STARTING) {
        s->state=UAC_HEALTH_RECOVERING; s->flushed=false; s->wait_turns=0;
    }
}
/* Return permission to clear the halted endpoint and refill the complete pool.
 * Never authorize reuse while any old transfer remains owned by the driver. */
static inline bool uac_recovery_retry(uac_recovery_t *s, bool gone, bool busy) {
    if (gone) { s->state=UAC_HEALTH_STOPPING; return false; }
    if (s->state != UAC_HEALTH_RECOVERING || !s->flushed) return false;
    if (busy) {
        if (++s->wait_turns >= UAC_DRAIN_TURNS) s->state=UAC_HEALTH_FAULT_HELD;
        return false;
    }
    if (s->attempts >= UAC_RECOVERY_LIMIT) { s->state=UAC_HEALTH_FAULT_HELD; return false; }
    ++s->attempts; s->state=UAC_HEALTH_STARTING; return true;
}
