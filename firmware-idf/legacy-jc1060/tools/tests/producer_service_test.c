#include <assert.h>
#include "producer_service.h"
int main(void) {
    producer_service_t s={.generation=7};
    producer_service_loop(&s,7,true,100,170,200);
    assert(s.handle.total_us==70 && s.tail.total_us==30 && s.handle.count==1);
    producer_service_loop(&s,7,false,200,300,400);
    producer_service_loop(&s,6,true,200,300,400);
    assert(s.handle.count==1);
    producer_service_loop(&s,7,true,400,300,500);assert(s.invalid==1);
    producer_service_callback(&s,1000);assert(s.callback_gap.count==0);
    producer_service_callback(&s,7000);assert(s.callback_gap.max_us==6000);
    producer_service_callback(&s,7010);assert(s.callback_gap.total_us==6010);
    producer_service_callback(&s,1);assert(s.invalid==2);
    s.handle.count=UINT32_MAX;s.handle.total_us=UINT64_MAX;
    producer_service_loop(&s,7,true,0,10,20);
    assert(s.saturated && s.handle.count==UINT32_MAX && s.handle.total_us==UINT64_MAX);
    const uint32_t words[]={1,2};assert(producer_profile_u64(words)==UINT64_C(8589934593));
    s=(producer_service_t){.generation=8};producer_service_callback(&s,100);
    assert(s.callback_gap.count==0 && s.invalid==0 && s.saturated==0);
    return 0;
}
