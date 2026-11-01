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
    return 0;
}
