#include <assert.h>
#include "uac_profile_record.h"
static int64_t now;
static unsigned core;
static bool inject;
unsigned uac_test_core(void) { return core; }
int64_t esp_timer_get_time(void) {
    if(inject) { inject=false;__wrap__global_interrupt_handler(0,1); }
    return now;
}
void __real__global_interrupt_handler(intptr_t sp,int cause) {
    (void)sp;
    assert(!ruzina_uac_irq_stamp().valid);
    now+=10;
    if(cause==2) __wrap__global_interrupt_handler(0,1);
    now+=10;
}
int main(void) {
    uac_irq_stamp_t a=ruzina_uac_irq_stamp();assert(a.valid);
    __wrap__global_interrupt_handler(0,1);
    uac_irq_stamp_t b=ruzina_uac_irq_stamp();assert(b.valid);
    uac_profile_record_t r={0};uac_profile_interrupts(&r,a,b);
    assert(r.irq_valid && r.irq_us==20 && r.irq_entries==1 && r.irq_nested==0);
    a=b;__wrap__global_interrupt_handler(0,2);b=ruzina_uac_irq_stamp();
    uac_profile_interrupts(&r,a,b);
    assert(!r.irq_valid && r.irq_nested==1 && r.irq_entries==2 && r.irq_us==60 && r.irq_window_us==40);
    core=1;a=ruzina_uac_irq_stamp();assert(!a.valid);
    __wrap__global_interrupt_handler(0,1);core=0;b=ruzina_uac_irq_stamp();
    assert(b.us==a.us && b.entries==a.entries);
    now=UINT32_MAX-5;a=ruzina_uac_irq_stamp();
    __wrap__global_interrupt_handler(0,1);b=ruzina_uac_irq_stamp();
    uac_profile_interrupts(&r,a,b);assert(r.irq_valid && r.irq_us==20 && r.irq_window_us==20);
    inject=true;assert(!ruzina_uac_irq_stamp().valid);
    assert(ruzina_uac_irq_stamp().valid);
    return 0;
}
