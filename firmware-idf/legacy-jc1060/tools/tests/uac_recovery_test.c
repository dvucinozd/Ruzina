#include <assert.h>
#include "uac_recovery.h"
int main(void) {
    uac_recovery_t s;
    uac_recovery_begin(&s, 7);
    assert(uac_recovery_current(&s, 7)); assert(!uac_recovery_current(&s, 6));
    /* Partial startup/fill/submit errors use the same recovery transition. */
    for (unsigned i=0; i<UAC_RECOVERY_LIMIT; ++i) {
        uac_recovery_fault(&s); assert(s.state==UAC_HEALTH_RECOVERING);
        assert(!uac_recovery_retry(&s, false, false));
        s.flushed=true;
        assert(!uac_recovery_retry(&s, false, true));
        assert(uac_recovery_retry(&s, false, false));
        assert(s.state==UAC_HEALTH_STARTING);
        s.state=UAC_HEALTH_STREAMING;
    }
    uac_recovery_fault(&s); s.flushed=true;
    assert(!uac_recovery_retry(&s, false, false)); assert(s.state==UAC_HEALTH_FAULT_HELD);
    uac_recovery_begin(&s, 8); uac_recovery_fault(&s); s.flushed=true;
    for(unsigned i=0;i<UAC_DRAIN_TURNS;++i) assert(!uac_recovery_retry(&s,false,true));
    assert(s.state==UAC_HEALTH_FAULT_HELD); // old resources cannot be reused
    uac_recovery_begin(&s, 9); uac_recovery_fault(&s); s.flushed=true;
    assert(!uac_recovery_retry(&s,true,false)); assert(s.state==UAC_HEALTH_STOPPING);
    assert(!uac_recovery_current(&s, 8));
    /* A stream that runs cleanly earns its recovery budget back. */
    uac_recovery_begin(&s, 10);
    for (unsigned i=0; i<UAC_RECOVERY_LIMIT; ++i) {
        uac_recovery_fault(&s); s.flushed=true;
        assert(uac_recovery_retry(&s, false, false)); s.state=UAC_HEALTH_STREAMING;
    }
    for (unsigned i=1; i<UAC_RECOVERY_HEALTHY_TRANSFERS; ++i) uac_recovery_healthy(&s);
    assert(s.attempts==UAC_RECOVERY_LIMIT);
    uac_recovery_healthy(&s); assert(s.attempts==0);
    /* A fault part-way through restarts the clean count. */
    uac_recovery_fault(&s); s.flushed=true;
    assert(uac_recovery_retry(&s, false, false)); s.state=UAC_HEALTH_STREAMING;
    for (unsigned i=1; i<UAC_RECOVERY_HEALTHY_TRANSFERS; ++i) uac_recovery_healthy(&s);
    uac_recovery_fault(&s); s.flushed=true;
    assert(uac_recovery_retry(&s, false, false)); s.state=UAC_HEALTH_STREAMING;
    for (unsigned i=1; i<UAC_RECOVERY_HEALTHY_TRANSFERS; ++i) uac_recovery_healthy(&s);
    assert(s.attempts==2);
    /* Healthy reports outside streaming change nothing. */
    s.state=UAC_HEALTH_RECOVERING; uac_recovery_healthy(&s); assert(s.attempts==2);
    return 0;
}
