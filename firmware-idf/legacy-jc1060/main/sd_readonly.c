#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "driver/sdmmc_host.h"
#include "esp_heap_caps.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "sdmmc_cmd.h"
#include "sd_pwr_ctrl_by_on_chip_ldo.h"
#include "ff.h"
#include "diskio.h"

_Static_assert(FF_FS_READONLY == 1 && FF_FS_EXFAT == 1 && FF_USE_MKFS == 0 && FF_USE_TRIM == 0,
               "SD probe must be read-only with exFAT and no format/trim");

static sdmmc_card_t card;
static FATFS filesystem;
static sd_pwr_ctrl_handle_t power;
static bool ready;
static uint32_t read_calls, denied_writes, denied_commands;
static esp_err_t (*host_transaction)(int, sdmmc_command_t *);

static esp_err_t readonly_transaction(int slot, sdmmc_command_t *cmd)
{
    /* Reject memory programming/erase/lock/general-write commands. Card init
     * still sends volatile bus-width/speed/app-command configuration. */
    switch (cmd->opcode) {
        case 24: case 25: case 27: case 28: case 29:
        case 32: case 33: case 38: case 42: case 56:
            denied_commands++;
            return ESP_ERR_NOT_SUPPORTED;
        default: return host_transaction(slot, cmd);
    }
}

void *ff_memalloc(size_t size) { return heap_caps_malloc(size, MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT); }
void ff_memfree(void *pointer) { heap_caps_free(pointer); }
DSTATUS disk_initialize(BYTE drive) { return drive == 0 && ready ? STA_PROTECT : STA_NOINIT; }
DSTATUS disk_status(BYTE drive) { return disk_initialize(drive); }
DRESULT disk_read(BYTE drive, BYTE *buffer, LBA_t sector, UINT count)
{
    if (drive || !ready) return RES_NOTRDY;
    if (!count || sector >= card.csd.capacity || count > card.csd.capacity - sector) return RES_PARERR;
    read_calls++;
    return sdmmc_read_sectors(&card, buffer, sector, count) == ESP_OK ? RES_OK : RES_ERROR;
}
DRESULT disk_write(BYTE drive, const BYTE *buffer, LBA_t sector, UINT count)
{
    (void)drive; (void)buffer; (void)sector; (void)count;
    denied_writes++;
    return RES_WRPRT;
}
DRESULT disk_ioctl(BYTE drive, BYTE command, void *buffer)
{
    if (drive || !ready) return RES_NOTRDY;
    switch (command) {
        case GET_SECTOR_COUNT: *(LBA_t *)buffer = card.csd.capacity; return RES_OK;
        case GET_SECTOR_SIZE: *(WORD *)buffer = 512; return RES_OK;
        case GET_BLOCK_SIZE: *(DWORD *)buffer = 1; return RES_OK;
        case CTRL_SYNC: return RES_OK; /* no pending write exists */
        default: return RES_WRPRT; /* includes TRIM, format, eject and power */
    }
}

static uint32_t hash_bytes(uint32_t hash, const void *data, size_t size)
{
    const uint8_t *p=data;
    while (size--) hash=(hash ^ *p++) * 16777619u;
    return hash;
}

static void sd_owner(void *unused)
{
    (void)unused;
    bool host_up=false, mounted=false;
    uint8_t *sample=NULL;
    uint32_t generation=1;
    esp_err_t err;
    sd_pwr_ctrl_ldo_config_t ldo = {.ldo_chan_id=4};
    err=sd_pwr_ctrl_new_on_chip_ldo(&ldo, &power);
    if (err!=ESP_OK) goto finish;
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
    err=sdmmc_host_init_slot(host.slot, &slot); if(err!=ESP_OK) goto finish;
    err=sdmmc_card_init(&host, &card); if(err!=ESP_OK) goto finish;
    if (card.csd.sector_size!=512) {err=ESP_ERR_NOT_SUPPORTED;goto finish;}
    ready=true;
    printf("S6_SD_ID capacity_bytes=%" PRIu64 " sectors=%" PRIu32 " sector_bytes=%d max_khz=10000 width=4 ldo=4 readonly=1\n",
           (uint64_t)card.csd.capacity * card.csd.sector_size, (uint32_t)card.csd.capacity, card.csd.sector_size);
    sample=heap_caps_malloc(4096, MALLOC_CAP_INTERNAL | MALLOC_CAP_DMA);
    if(!sample) {err=ESP_ERR_NO_MEM;goto finish;}
    if(disk_read(0,sample,0,8)!=RES_OK) {err=ESP_FAIL;goto finish;}
    uint32_t first_hash=hash_bytes(2166136261u,sample,4096);
    if(disk_read(0,sample,0,8)!=RES_OK || hash_bytes(2166136261u,sample,4096)!=first_hash) {err=ESP_FAIL;goto finish;}
    /* Self-test the write boundary without submitting a host command. */
    if(disk_write(0,sample,0,1)!=RES_WRPRT || disk_ioctl(0,CTRL_TRIM,NULL)!=RES_WRPRT) {err=ESP_FAIL;goto finish;}
    FRESULT fr=f_mount(&filesystem,"0:",1);
    if(fr!=FR_OK) {printf("sd_mount_fresult=%u\n",(unsigned)fr);err=ESP_FAIL;goto finish;}
    mounted=true;
    printf("S6_SD_MOUNT fs_type=%u exfat=%u cluster_sectors=%" PRIu32 " volume_lba=%" PRIu32 " generation=%" PRIu32 "\n",
           filesystem.fs_type, filesystem.fs_type==FS_EXFAT, (uint32_t)filesystem.csize,(uint32_t)filesystem.volbase,generation);
    if(filesystem.fs_type!=FS_EXFAT) {err=ESP_ERR_NOT_SUPPORTED;goto finish;}
    FF_DIR dir; FILINFO entry;
    fr=f_opendir(&dir,"0:/");
    if(fr!=FR_OK) {err=ESP_FAIL;goto finish;}
    unsigned files=0,dirs=0,entries=0; bool complete=false, sampled=false;
    for(;entries<64;++entries) {
        fr=f_readdir(&dir,&entry);
        if(fr!=FR_OK) break;
        if(!entry.fname[0]) {complete=true;break;}
        if(entry.fattrib & AM_DIR) dirs++;
        else {
            files++;
            if(!sampled) {
                char path[260]; FIL file;
                int length=snprintf(path,sizeof(path),"0:/%s",entry.fname);
                if(length>0 && length<(int)sizeof(path) && f_open(&file,path,FA_READ)==FR_OK) {
                    uint32_t hash=2166136261u,total=0; UINT got=0;
                    do {
                        fr=f_read(&file,sample,4096,&got);
                        if(fr!=FR_OK) break;
                        hash=hash_bytes(hash,sample,got);total+=got;
                    } while(got && total<65536);
                    FRESULT close_result=f_close(&file);
                    if(fr!=FR_OK || close_result!=FR_OK) break;
                    printf("sd_file_prefix_bytes=%" PRIu32 " fnv1a=%08" PRIx32 " name=NOT_LOGGED\n",total,hash);
                    sampled=true;
                }
            }
        }
    }
    FRESULT close_dir=f_closedir(&dir);
    if(fr!=FR_OK || close_dir!=FR_OK) {err=ESP_FAIL;goto finish;}
    printf("S6_SD_ROOT files=%u directories=%u enumeration_complete=%u file_read=%s first8sectors_fnv1a=%08" PRIx32 "\n",
           files,dirs,complete,sampled?"PREFIX_OBSERVED":"NOT_RUN_NO_ROOT_FILE",first_hash);
#if CONFIG_RUZINA_SD_FIXTURE
    extern esp_err_t ruzina_sd_fixture_verify(uint8_t *, uint32_t, uint32_t);
    uint32_t fixture_pass=1;
    err=ruzina_sd_fixture_verify(sample,fixture_pass,generation);
    if(err!=ESP_OK) goto finish;
#endif
    printf("sd_result=READONLY_EXFAT_MOUNT_ROOT_OBSERVED write_boundary_rejections=%" PRIu32 " host_denied=%" PRIu32 " generation=%" PRIu32 "\n",
           denied_writes,denied_commands,generation);
    fflush(stdout);
    for(unsigned tick=1;;++tick) {
        vTaskDelay(pdMS_TO_TICKS(1000));
        err=sdmmc_get_status(&card);
        if(err!=ESP_OK || disk_read(0,sample,0,8)!=RES_OK) {if(err==ESP_OK)err=ESP_FAIL;break;}
        if(hash_bytes(2166136261u,sample,4096)!=first_hash) {err=ESP_ERR_INVALID_STATE;break;}
#if CONFIG_RUZINA_SD_FIXTURE
        err=ruzina_sd_fixture_verify(sample,++fixture_pass,generation);
        if(err!=ESP_OK) break;
#endif
        if(tick%5==0) printf("sd_health generation=%" PRIu32 " reads=%" PRIu32 " write_calls=%" PRIu32 " host_denied=%" PRIu32 " stack_free=%u\n",
            generation,read_calls,denied_writes,denied_commands,(unsigned)uxTaskGetStackHighWaterMark(NULL));
    }
finish:
    /* One synchronous owner: invalidate generation before unmount/deinit.
     * No queued IO/borrowed handles escape. Reinsert requires explicit restart. */
    ready=false;generation++;
    if(mounted) f_mount(NULL,"0:",0);
    if(host_up) sdmmc_host_deinit();
    if(power) {sd_pwr_ctrl_del_on_chip_ldo(power);power=NULL;}
    heap_caps_free(sample);
    printf("sd_state=QUIESCED error=%s generation=%" PRIu32 " reads=%" PRIu32 " write_boundary_rejections=%" PRIu32 " host_denied=%" PRIu32 " reinsert=RESTART_REQUIRED\n",
           esp_err_to_name(err),generation,read_calls,denied_writes,denied_commands);
    fflush(stdout);
    vTaskDelete(NULL);
}

esp_err_t ruzina_sd_readonly_start(void)
{
    return xTaskCreate(sd_owner,"sd_readonly",8192,NULL,1,NULL)==pdPASS ? ESP_OK : ESP_ERR_NO_MEM;
}
