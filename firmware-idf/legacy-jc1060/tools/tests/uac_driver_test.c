/* Compile and fault-inject the actual driver included by usb_host_enum.c. */
#include <assert.h>
#include <inttypes.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "uac_recovery.h"
#define CONFIG_RUZINA_DECK_MIX 0
#define CONFIG_RUZINA_DECK_PLAY 0
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
static unsigned char log_storage[256];
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
    (void)b;(void)c;if(fill_failures){--fill_failures;return 0;}for(unsigned i=0;i<packets;++i)sizes[i]=384;return packets*384;
}
void ruzina_legacy_uac_stats(uint32_t *out) { memset(out,0,3*sizeof(*out)); }
static void start(void) {
    usb_config_desc_t config={0};gone=false;device=1;
    uac_start(&config,USB_SPEED_HIGH); assert(uac_ctrl_busy);
    complete(uac_ctrl,USB_TRANSFER_STATUS_COMPLETED);
}
static void stop(void) { gone=true;assert(uac_stop());assert(live==0);assert(frees==allocations);gone=false; }
int main(void) {
    start();assert(live==4);
    uac_report_pending=true;uac_report();assert(log_waiting==1);
    uac_report_pending=true;uac_report();assert(uac_log_drops==1);
    uac_log_snapshot_t snapshot;assert(xQueueReceive(uac_log_queue,&snapshot,0)==1);
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
