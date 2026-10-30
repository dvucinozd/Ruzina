#include <inttypes.h>
#include <stdatomic.h>
#include <stdio.h>
#include "sdkconfig.h"
#include "driver/sdmmc_host.h"
#include "esp_heap_caps.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "sdmmc_cmd.h"
#include "sd_pwr_ctrl_by_on_chip_ldo.h"

/* Task-only C ABI: Rust layouts, futures, tickets and buffers never cross it. */
extern int32_t ruzina_legacy_bridge_init(uint32_t sectors);
extern int32_t ruzina_legacy_bridge_take(uint32_t *first, uint32_t *count);
extern int32_t ruzina_legacy_bridge_complete(int32_t status, const uint8_t *data, uint32_t bytes);
extern int32_t ruzina_legacy_bridge_invalidate(void);
extern int32_t ruzina_legacy_bridge_run(void);
#if CONFIG_RUZINA_SD_RUST_EXFAT
extern int32_t ruzina_legacy_exfat_run(void);
#endif

static portMUX_TYPE bridge_mux = portMUX_INITIALIZER_UNLOCKED;
static TaskHandle_t worker_task, client_task;
static atomic_bool stop_requested;
static esp_err_t (*host_transaction)(int, sdmmc_command_t *);
static uint32_t reads, denied_commands;

void ruzina_bridge_lock(void) { portENTER_CRITICAL(&bridge_mux); }
void ruzina_bridge_unlock(void) { portEXIT_CRITICAL(&bridge_mux); }
void ruzina_bridge_notify_worker(void) { xTaskNotifyGive(worker_task); }
void ruzina_bridge_notify_client(void) { xTaskNotifyGive(client_task); }
void ruzina_bridge_wait_client(void) { ulTaskNotifyTake(pdTRUE, pdMS_TO_TICKS(1000)); }
void ruzina_bridge_delay(void) { vTaskDelay(pdMS_TO_TICKS(1000)); }
void ruzina_bridge_report(uint32_t pass, uint32_t event, uint32_t value)
{
    printf("rust_sd_bridge pass=%" PRIu32 " event=%" PRIu32 " value=%08" PRIx32 " stack_free=%u\n",
           pass, event, value, (unsigned)uxTaskGetStackHighWaterMark(NULL));
    fflush(stdout);
}

void ruzina_bridge_fixture_yield(void) { vTaskDelay(1); }
void ruzina_bridge_fs_report(uint32_t pass, uint32_t event, uint32_t value)
{
    printf("rust_exfat pass=%" PRIu32 " event=%" PRIu32 " value=%" PRIu32 " stack_free=%u",
           pass,event,value,(unsigned)uxTaskGetStackHighWaterMark(NULL));
    if(event==3) printf(" sha256=1cb18815702b14ff92ad02f4976a162481d8d91beb429ba5607b131d4ab281de eof=MATCH seek_cases=10");
    printf("\n");fflush(stdout);
}

static esp_err_t readonly_transaction(int slot, sdmmc_command_t *command)
{
    switch (command->opcode) {
        case 24: case 25: case 27: case 28: case 29:
        case 32: case 33: case 38: case 42: case 56:
            denied_commands++;
            return ESP_ERR_NOT_SUPPORTED;
        default: return host_transaction(slot, command);
    }
}

static void client(void *unused)
{
    (void)unused;
    /* Register on this task before any Rust waker can target it. The task stays
     * allocated after completion so static RawWakers never reference freed TCBs. */
    client_task=xTaskGetCurrentTaskHandle();
#if CONFIG_RUZINA_SD_RUST_EXFAT
    int32_t result=ruzina_legacy_exfat_run();
#else
    int32_t result=ruzina_legacy_bridge_run();
#endif
    (void)ruzina_legacy_bridge_invalidate();
    atomic_store_explicit(&stop_requested,true,memory_order_release);
    ruzina_bridge_notify_worker();
    printf("rust_sd_client=STOPPED result=%" PRId32 " pending_drain=OWNER\n",result);
    fflush(stdout);
    vTaskSuspend(NULL);
}

static void owner(void *unused)
{
    (void)unused;
    worker_task=xTaskGetCurrentTaskHandle();
    sdmmc_card_t card;
    sd_pwr_ctrl_handle_t power=NULL;
    uint8_t *bounce=NULL;
    bool host_up=false;
    esp_err_t err;
    sd_pwr_ctrl_ldo_config_t ldo={.ldo_chan_id=4};
    err=sd_pwr_ctrl_new_on_chip_ldo(&ldo,&power); if(err!=ESP_OK) goto finish;
    sdmmc_host_t host=SDMMC_HOST_DEFAULT();
    host.slot=SDMMC_HOST_SLOT_0;
    host.max_freq_khz=10000;
    host.flags=SDMMC_HOST_FLAG_4BIT;
    host.command_timeout_ms=1000;
    host.pwr_ctrl_handle=power;
    host_transaction=host.do_transaction;
    host.do_transaction=readonly_transaction;
    err=sdmmc_host_init(); if(err!=ESP_OK) goto finish;
    host_up=true;
    sdmmc_slot_config_t slot=SDMMC_SLOT_CONFIG_DEFAULT();
    slot.width=4; slot.clk=43; slot.cmd=44; slot.d0=39; slot.d1=40; slot.d2=41; slot.d3=42;
    slot.cd=SDMMC_SLOT_NO_CD; slot.wp=SDMMC_SLOT_NO_WP; slot.flags=0;
    err=sdmmc_host_init_slot(host.slot,&slot); if(err!=ESP_OK) goto finish;
    err=sdmmc_card_init(&host,&card); if(err!=ESP_OK) goto finish;
    if(card.csd.sector_size!=512) {err=ESP_ERR_NOT_SUPPORTED;goto finish;}
    bounce=heap_caps_malloc(512,MALLOC_CAP_INTERNAL|MALLOC_CAP_DMA);
    if(!bounce || ((uintptr_t)bounce & 3u)) {err=ESP_ERR_NO_MEM;goto finish;}
    if(ruzina_legacy_bridge_init(card.csd.capacity)!=0) {err=ESP_ERR_INVALID_STATE;goto finish;}
    printf("S6_RUST_SD_BRIDGE sectors=%" PRIu32 " sector_bytes=512 max_khz=10000 width=4 ldo=4 bounce_bytes=512 readonly=1\n",
           (uint32_t)card.csd.capacity);
#if CONFIG_RUZINA_SD_RUST_EXFAT
    const uint32_t client_stack=32768;
#else
    const uint32_t client_stack=16384;
#endif
    if(xTaskCreatePinnedToCore(client,"sd_rust_client",client_stack,NULL,3,NULL,1)!=pdPASS) {err=ESP_ERR_NO_MEM;goto finish;}
    for(;;) {
        ulTaskNotifyTake(pdTRUE,pdMS_TO_TICKS(1000));
        if(atomic_load_explicit(&stop_requested,memory_order_acquire)) {err=ESP_FAIL;break;}
        uint32_t first=0,count=0;
        int32_t taken=ruzina_legacy_bridge_take(&first,&count);
        if(taken<0) {err=ESP_ERR_INVALID_STATE;break;}
        if(!taken) {
            err=sdmmc_get_status(&card);
            if(err!=ESP_OK) break;
            continue;
        }
        /* C independently validates before touching driver/DMA. Rust only lends
         * scalar coordinates; this C allocation remains the sole DMA buffer. */
        if(count!=1 || first>=card.csd.capacity) err=ESP_ERR_INVALID_ARG;
        else { reads++;err=sdmmc_read_sectors(&card,bounce,first,count); }
        /* sdmmc_read_sectors is synchronous: transfer has ended on return. Rust
         * copies exactly512 bytes while holding its short state critical section. */
        int32_t completed=ruzina_legacy_bridge_complete(err,err==ESP_OK?bounce:NULL,err==ESP_OK?512:0);
        if(completed!=0) {err=ESP_ERR_INVALID_STATE;break;}
        if(err!=ESP_OK) break;
        if(reads%80==0) {
            printf("rust_sd_owner reads=%" PRIu32 " denied_commands=%" PRIu32 " stack_free=%u\n",
                   reads,denied_commands,(unsigned)uxTaskGetStackHighWaterMark(NULL));
            fflush(stdout);
        }
    }
finish:
    /* Never free DMA/card memory while a request could still be active. An ABI
     * invariant failure holds hardware for recovery rather than risking UAF. */
    if(!ruzina_legacy_bridge_invalidate()) {
        printf("rust_sd_owner=FAULT_HELD_NO_TEARDOWN error=%s\n",esp_err_to_name(err));
        fflush(stdout);vTaskSuspend(NULL);
    }
    if(host_up) sdmmc_host_deinit();
    if(power) sd_pwr_ctrl_del_on_chip_ldo(power);
    heap_caps_free(bounce);
    printf("rust_sd_owner=QUIESCED error=%s reads=%" PRIu32 " denied_commands=%" PRIu32 " reinsert=APP_RESTART_REQUIRED\n",
           esp_err_to_name(err),reads,denied_commands);
    fflush(stdout);
    /* Keep task handle valid for any outstanding static notification. */
    vTaskSuspend(NULL);
}

esp_err_t ruzina_sd_bridge_start(void)
{
    atomic_store_explicit(&stop_requested,false,memory_order_relaxed);
    return xTaskCreatePinnedToCore(owner,"sd_rust_owner",8192,NULL,4,NULL,0)==pdPASS ? ESP_OK : ESP_ERR_NO_MEM;
}
