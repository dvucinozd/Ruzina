#include <assert.h>
#include <stdint.h>
#include "uac_profile_sched.h"
static int64_t now;
static bool inject_switch;
int64_t esp_timer_get_time(void) {
    if (inject_switch) { inject_switch=false;ruzina_uac_switched_out((void *)1,0);now+=20;ruzina_uac_switched_in((void *)1,0); }
    return now;
}
int main(void) {
    assert(!ruzina_uac_sched_stamp().valid);
    assert(!ruzina_uac_sched_bind(0));
    assert(ruzina_uac_sched_bind((void *)1));assert(ruzina_uac_sched_bind((void *)1));
    assert(!ruzina_uac_sched_bind((void *)2));
    now=10;uac_sched_stamp_t a=ruzina_uac_sched_stamp();assert(a.valid);
    ruzina_uac_switched_out((void *)2,0);ruzina_uac_switched_in((void *)2,0);
    ruzina_uac_switched_out((void *)1,1);ruzina_uac_switched_in((void *)1,1);
    now=20;ruzina_uac_switched_out((void *)1,0);
    assert(!ruzina_uac_sched_stamp().valid);
    now=10020;ruzina_uac_switched_in((void *)2,0);
    assert(!ruzina_uac_sched_stamp().valid);
    ruzina_uac_switched_in((void *)1,0);
    uac_sched_stamp_t b=ruzina_uac_sched_stamp();
    assert(b.valid && b.away_us-a.away_us==10000 && b.switches-a.switches==1);
    now=UINT32_MAX-5;ruzina_uac_switched_out((void *)1,0);
    now=(int64_t)UINT32_MAX+15;ruzina_uac_switched_in((void *)1,0);
    uac_sched_stamp_t c=ruzina_uac_sched_stamp();assert(c.valid && c.away_us-b.away_us==20);
    inject_switch=true;assert(!ruzina_uac_sched_stamp().valid);
    assert(ruzina_uac_sched_stamp().valid);
    ruzina_uac_catalog_event(4);assert(ruzina_uac_catalog_phase()==4);
    ruzina_uac_catalog_event(13);assert(ruzina_uac_catalog_phase()==4);
    return 0;
}
