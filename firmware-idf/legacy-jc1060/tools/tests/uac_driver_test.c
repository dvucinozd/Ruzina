/* Compile and fault-inject the actual driver included by usb_host_enum.c. */
#include <assert.h>
#include <inttypes.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "uac_recovery.h"
#ifndef UAC_TEST_TIMING
#define UAC_TEST_TIMING 0
#endif
#define CONFIG_RUZINA_DECK_MIX UAC_TEST_TIMING
#define CONFIG_RUZINA_DECK_PLAY UAC_TEST_TIMING
#define CONFIG_RUZINA_DECK_ENGINE UAC_TEST_TIMING
#ifndef UAC_TEST_PROFILE
#define UAC_TEST_PROFILE 0
#endif
#define CONFIG_RUZINA_UAC_PROFILE UAC_TEST_PROFILE
#define ESP_OK 0
#define USB_TRANSFER_STATUS_COMPLETED 0
#define USB_TRANSFER_STATUS_NO_DEVICE 1
#define USB_TRANSFER_STATUS_CANCELED 2
#define USB_SPEED_HIGH 2
#define pdMS_TO_TICKS(x) (x)
typedef int esp_err_t;
typedef int usb_speed_t;
typedef struct { uint16_t wTotalLength; } usb_config_desc_t;
typedef struct usb_transfer {
    unsigned char *data_buffer; int data_buffer_size, num_bytes;
    int device_handle, bEndpointAddress, status;
    void (*callback)(struct usb_transfer *);
    void *context;
    struct {int num_bytes,status;} isoc_packet_desc[6];
} usb_transfer_t;
static int client=1, device=1;
static bool gone;
static int alloc_fail_after=-1, submit_failures, fill_failures, endpoint_error;
static int allocations, frees, submitted, live, flushes, clears;
static bool hold_callbacks;
#if UAC_TEST_PROFILE
static const void *xTaskGetCurrentTaskHandle(void) { return (void *)1; }
static unsigned uxTaskGetStackHighWaterMark(const void *task) { (void)task;return 3072; }
#endif
#if UAC_TEST_TIMING
static int64_t mock_time, fill_duration=200;
static uint32_t fill_calls;
static const uint32_t *fill_sequence;
static unsigned fill_sequence_count, fill_sequence_pos;
int64_t esp_timer_get_time(void) { return mock_time; }
static void sequence(const uint32_t *values, unsigned count) {
    fill_sequence=values; fill_sequence_count=count; fill_sequence_pos=0;
}
#endif
static usb_transfer_t *owned[16];
static const char *esp_err_to_name(int e) { return e ? "FAIL" : "OK"; }
static int usb_host_transfer_alloc(int bytes,int packets,usb_transfer_t **out) {
    (void)packets;
    if(alloc_fail_after==0) return -1;
    if(alloc_fail_after>0) --alloc_fail_after;
    *out=calloc(1,sizeof(**out)); (*out)->data_buffer=calloc(1,(size_t)bytes); (*out)->data_buffer_size=bytes;
    allocations++; return 0;
}
static int usb_host_transfer_submit(usb_transfer_t *t) {
    assert(device && !gone);
    if(submit_failures) {--submit_failures;return -1;}
    for(unsigned i=0;i<16;++i) if(!owned[i]) {owned[i]=t;live++;submitted++;return 0;}
    assert(false); return -1;
}
static int usb_host_transfer_submit_control(int c,usb_transfer_t *t) { (void)c; return usb_host_transfer_submit(t); }
static int usb_host_transfer_free(usb_transfer_t *t) {
    for(unsigned i=0;i<16;++i) assert(owned[i]!=t);
    free(t->data_buffer); free(t); frees++;return 0;
}
static void complete(usb_transfer_t *t,int status) {
    bool found=false;
    for(unsigned i=0;i<16;++i) if(owned[i]==t) {owned[i]=NULL;live--;found=true;break;}
    assert(found);t->status=status;t->callback(t);
}
static int usb_host_endpoint_halt(int d,int ep) { (void)d;(void)ep;return endpoint_error; }
static int usb_host_endpoint_flush(int d,int ep) { (void)d;(void)ep;flushes++;return endpoint_error; }
static int usb_host_endpoint_clear(int d,int ep) { (void)d;(void)ep;clears++;return endpoint_error; }
static int usb_host_interface_claim(int c,int d,int i,int a) { (void)c;(void)d;(void)i;(void)a;return 0; }
static int usb_host_interface_release(int c,int d,int i) { (void)c;(void)d;(void)i;assert(live==0);return 0; }
static int usb_host_client_handle_events(int c,int ticks) {
    (void)c;(void)ticks;
    if(!hold_callbacks) for(unsigned i=0;i<16;++i) if(owned[i]) complete(owned[i],gone?USB_TRANSFER_STATUS_NO_DEVICE:USB_TRANSFER_STATUS_CANCELED);
    return 0;
}
typedef void *QueueHandle_t;
#define pdTRUE 1
#define pdPASS 1
#define portMAX_DELAY -1
static unsigned char log_storage[1024];
static unsigned log_size, log_waiting;
static QueueHandle_t xQueueCreate(unsigned count,unsigned bytes){assert(count==1 && bytes<=sizeof(log_storage));log_size=bytes;return log_storage;}
static int xQueueReceive(QueueHandle_t q,void *out,int wait){(void)q;(void)wait;if(!log_waiting)return 0;memcpy(out,log_storage,log_size);log_waiting=0;return 1;}
static unsigned uxQueueMessagesWaiting(QueueHandle_t q){(void)q;return log_waiting;}
static int xQueueOverwrite(QueueHandle_t q,const void *sample){(void)q;memcpy(log_storage,sample,log_size);log_waiting=1;return 1;}
static void vQueueDelete(QueueHandle_t q){(void)q;}
static int xTaskCreatePinnedToCore(void (*fn)(void *),const char *name,unsigned stack,void *arg,int priority,void *handle,int core){(void)fn;(void)name;(void)arg;(void)handle;(void)core;assert(stack==3072 && priority==1);return 1;}
#include "uac_driver.inc"
int32_t ruzina_legacy_uac_select(const uint8_t *c,uint32_t l,int32_t h,uint32_t *o) {
    (void)c;(void)l;(void)h;uint32_t info[8]={1,1,2,384,4,2,48000,0};memcpy(o,info,sizeof(info));return 1;
}
uint32_t ruzina_legacy_uac_control(uint32_t step,uint8_t *out) { (void)step;out[0]=0;return 8; }
uint32_t ruzina_legacy_uac_fill(uint8_t *b,uint32_t c,uint32_t *sizes,uint32_t packets) {
#if UAC_TEST_TIMING
    mock_time += fill_sequence_pos < fill_sequence_count ? fill_sequence[fill_sequence_pos++] : fill_duration;
    ++fill_calls;
#endif
    (void)b;(void)c;if(fill_failures){--fill_failures;return 0;}for(unsigned i=0;i<packets;++i)sizes[i]=384;return packets*384;
}
void ruzina_legacy_uac_stats(uint32_t *out) { memset(out,0,3*sizeof(*out)); }
#if UAC_TEST_TIMING
void ruzina_legacy_mix_stats(uint32_t *out) { memset(out,0,8*sizeof(*out)); }
void ruzina_legacy_play_diagnostics(uint32_t *out) { memset(out,0,4*sizeof(*out)); }
void ruzina_legacy_engine_status(uint32_t *out) { memset(out,0,4*sizeof(*out)); }
void ruzina_legacy_engine_counters(uint32_t *out) { memset(out,0,2*sizeof(*out)); }
#endif
#if UAC_TEST_PROFILE
static unsigned profile_begin_count,profile_end_count;
static int64_t profile_start;
void ruzina_legacy_uac_profile_begin(void) { profile_begin_count++;profile_start=mock_time; }
uint32_t ruzina_legacy_uac_profile_end(uint32_t *out,uint32_t len) {
    assert(len==UAC_PROFILE_WORDS); profile_end_count++;memset(out,0,len*sizeof(*out));
    out[0]=(uint32_t)(mock_time-profile_start);out[2]=6;out[3]=2;out[4+7]=out[0];out[12+7]=1;
    return len;
}
#endif
static void start(void) {
    usb_config_desc_t config={0};gone=false;device=1;
    uac_start(&config,USB_SPEED_HIGH); assert(uac_ctrl_busy);
    complete(uac_ctrl,USB_TRANSFER_STATUS_COMPLETED);
}
static void stop(void) { gone=true;assert(uac_stop());assert(live==0);assert(frees==allocations);gone=false; }
#if UAC_TEST_TIMING
static void timing_phases(void) {
    const uint32_t initial[]={12260,200,3000,3001}; sequence(initial,4);
    start(); assert(live==4);
    const uac_fill_phase_stats_t *p=&uac_fill_timing.phase[UAC_FILL_INITIAL_PREFILL];
    assert(p->count==4 && p->first_us==12260 && p->max_us==12260);
    assert(!p->first_after_iso_submit && !p->max_after_iso_submit);
    assert(p->gt_3000==2 && p->ge_6000==1 && p->failures==0);
    assert(uac_fill_timing.first_connection_us==12260);
    assert(uac_fill_timing.first_connection_phase==UAC_FILL_INITIAL_PREFILL);
    assert(!uac_fill_timing.first_connection_after_iso_submit);
    assert(uac_fill_timing.phase[UAC_FILL_RECOVERY_PREFILL].count==0);
    assert(uac_fill_timing.phase[UAC_FILL_CALLBACK].count==0);
    assert(uac_fill_max_us==12260);
    fill_duration=6000; complete(uac_iso[0],USB_TRANSFER_STATUS_COMPLETED);
    fill_failures=1; fill_duration=6001; complete(uac_iso[1],USB_TRANSFER_STATUS_COMPLETED);
    p=&uac_fill_timing.phase[UAC_FILL_CALLBACK];
    assert(p->count==2 && p->first_us==6000 && p->max_us==6001);
    assert(p->first_after_iso_submit && p->max_after_iso_submit);
    assert(p->gt_3000==2 && p->ge_6000==2 && p->failures==1);
    assert(uac_fill_max_us==12260);
    assert(uac_health.state==UAC_HEALTH_RECOVERING);
    uac_recover_poll(); usb_host_client_handle_events(1,0);
    const uint32_t recovery[]={5999,6000,3000,0}; sequence(recovery,4);
    uac_recover_poll(); assert(live==4);
    p=&uac_fill_timing.phase[UAC_FILL_RECOVERY_PREFILL];
    assert(p->count==4 && p->first_us==5999 && p->max_us==6000);
    assert(p->first_after_iso_submit && p->max_after_iso_submit);
    assert(p->gt_3000==2 && p->ge_6000==1 && p->failures==0);
    assert(uac_fill_timing.first_connection_us==12260);
    uac_report_pending=true; uac_report();
    uac_log_snapshot_t sample; assert(xQueueReceive(uac_log_queue,&sample,0)==1);
    assert(sample.fill_max_us==12260 && sample.generation==uac_generation);
    assert(sample.timing.phase[UAC_FILL_CALLBACK].failures==1);
    assert(sample.timing.phase[UAC_FILL_RECOVERY_PREFILL].count==4);
    uint32_t prior_generation=uac_generation; stop();
    fill_duration=55; start(); assert(uac_generation==prior_generation+1);
    assert(uac_fill_timing.first_connection_us==55);
    assert(!uac_fill_timing.first_connection_after_iso_submit);
    assert(uac_fill_timing.phase[UAC_FILL_INITIAL_PREFILL].count==8);
    assert(uac_fill_timing.phase[UAC_FILL_INITIAL_PREFILL].first_us==12260);
    assert(uac_fill_max_us==12260); stop();
    // Allocation failure before the first fill: the first connection sample
    // is a recovery prefill with no audio submission in this connection yet.
    alloc_fail_after=2; start(); assert(live==0);
    assert(!uac_fill_timing.first_connection_seen);
    alloc_fail_after=-1; uac_recover_poll(); uac_recover_poll(); assert(live==4);
    assert(uac_fill_timing.first_connection_phase==UAC_FILL_RECOVERY_PREFILL);
    assert(!uac_fill_timing.first_connection_after_iso_submit);
    uint32_t classified=0;
    for(unsigned i=0;i<UAC_FILL_PHASE_COUNT;++i) classified+=uac_fill_timing.phase[i].count;
    assert(classified==fill_calls);
    // An old callback produces no timing sample in a new session.
    uac_context_t stale={uac_generation-1,0}; usb_transfer_t fake={.context=&stale};
    uint32_t before=fill_calls; uac_iso_done(&fake); assert(fill_calls==before);
    // Telemetry must not wrap after a very long blocked call or counter limit.
    fill_duration=(int64_t)UINT32_MAX+42; complete(uac_iso[0],USB_TRANSFER_STATUS_COMPLETED);
    assert(uac_fill_max_us==UINT32_MAX);
    assert(uac_fill_timing.phase[UAC_FILL_CALLBACK].max_us==UINT32_MAX);
    stop(); fill_duration=200;
    uac_fill_timing_t saturated={0};
    saturated.phase[UAC_FILL_CALLBACK]=(uac_fill_phase_stats_t){.count=UINT32_MAX,.gt_3000=UINT32_MAX,.ge_6000=UINT32_MAX,.failures=UINT32_MAX};
    uac_timing_record(&saturated,UAC_FILL_CALLBACK,7000,true,true);
    p=&saturated.phase[UAC_FILL_CALLBACK];
    assert(p->count==UINT32_MAX && p->gt_3000==UINT32_MAX && p->ge_6000==UINT32_MAX && p->failures==UINT32_MAX);
    assert(p->max_us==7000);
#if UAC_TEST_PROFILE
    assert(profile_begin_count==profile_end_count && profile_begin_count==fill_calls);
    assert(uac_profile_worst.phase==UAC_FILL_CALLBACK && uac_profile_worst.rust_valid);
    assert(uac_profile_worst.generation==uac_generation && uac_profile_over_seen);
#endif
    // Leave the original fault campaign's independent counters untouched.
    flushes=clears=0; uac_log_drops=0;
}
#endif
int main(void) {
#if UAC_TEST_TIMING
    timing_phases();
#endif
    start();assert(live==4);
    uac_report_pending=true;uac_report();assert(log_waiting==1);
    uac_report_pending=true;uac_report();assert(uac_log_drops==1);
    uac_log_snapshot_t snapshot;assert(xQueueReceive(uac_log_queue,&snapshot,0)==1);
#if UAC_TEST_PROFILE
    assert(snapshot.log_drops==1);
    assert(snapshot.profile_seen && snapshot.worst.rust_valid);
    assert(snapshot.worst.generation<=snapshot.generation);
    // Pure interval arithmetic handles wrap and rejects incoherent samples.
    uac_profile_record_t interval={0};
    uac_profile_schedule(&interval,(uac_sched_stamp_t){.away_us=UINT32_MAX-9,.switches=UINT32_MAX,.wall_us=UINT32_MAX-29,.valid=true},
                        (uac_sched_stamp_t){.away_us=10,.switches=0,.wall_us=30,.valid=true});
    assert(interval.sched_valid && interval.sched_away_us==20 && interval.sched_switches==1 && interval.sched_window_us==60);
    uac_profile_schedule(&interval,(uac_sched_stamp_t){.valid=false},(uac_sched_stamp_t){.valid=true});
    assert(!interval.sched_valid);
#endif
    assert(!uac_report_pending);assert(uac_health.state==UAC_HEALTH_STREAMING);
    complete(uac_iso[0],9);assert(uac_health.state==UAC_HEALTH_RECOVERING);
    uac_recover_poll();assert(flushes==1);assert(live==3);
    usb_host_client_handle_events(1,0);assert(live==0);
    uac_recover_poll();assert(clears==1);assert(live==4);
    submit_failures=1;complete(uac_iso[0],0);assert(uac_submit_errors==1);
    uac_recover_poll();usb_host_client_handle_events(1,0);uac_recover_poll();assert(live==4);
    fill_failures=1;complete(uac_iso[1],0);assert(uac_fill_errors==1);
    uac_recover_poll();usb_host_client_handle_events(1,0);uac_recover_poll();assert(live==4);
    complete(uac_iso[2],9);uac_recover_poll();usb_host_client_handle_events(1,0);uac_recover_poll();
    assert(uac_health.state==UAC_HEALTH_FAULT_HELD);assert(live==0);stop();
    alloc_fail_after=2;start();assert(uac_health.state==UAC_HEALTH_RECOVERING);assert(live==0);
    alloc_fail_after=-1;uac_recover_poll();uac_recover_poll();assert(live==4);
    hold_callbacks=true;gone=true;int previous_frees=frees;
    assert(!uac_stop());assert(frees==previous_frees);assert(uac_interface!=0xff);
    hold_callbacks=false;assert(uac_stop());assert(frees==allocations);gone=false;
    // EP0 completion after loss cannot start streaming.
    usb_config_desc_t config={0};uac_start(&config,USB_SPEED_HIGH);gone=true;
    complete(uac_ctrl,USB_TRANSFER_STATUS_NO_DEVICE);assert(live==0);assert(uac_stop());gone=false;
    // A callback with an old generation cannot alter the new pool.
    start();uac_context_t stale={uac_generation-1,0};usb_transfer_t fake={.context=&stale};
    unsigned before=(unsigned)live;uac_iso_done(&fake);assert((unsigned)live==before);stop();
    return 0;
}
