#include <inttypes.h>
#include <stdio.h>
#include <string.h>
#include "esp_err.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "psa/crypto.h"
#include "ff.h"

#define FIXTURE_SIZE (8u * 1024u * 1024u + 513u)
static const uint8_t expected_sha[32] = {
    0x1c,0xb1,0x88,0x15,0x70,0x2b,0x14,0xff,0x92,0xad,0x02,0xf4,0x97,0x6a,0x16,0x24,
    0x81,0xd8,0xd9,0x1b,0xeb,0x42,0x9b,0xa5,0x60,0x7b,0x13,0x1d,0x4a,0xb2,0x81,0xde
};

static bool matches(const uint8_t *data, uint32_t offset, uint32_t length)
{
    for(uint32_t i=0;i<length;++i) {
        uint32_t p=offset+i;
        uint8_t expected=(uint8_t)(p*73u+(p>>8)*19u+(p>>17)*131u+0x5au);
        if(data[i]!=expected) return false;
    }
    return true;
}

esp_err_t ruzina_sd_fixture_verify(uint8_t *buffer, uint32_t pass, uint32_t generation)
{
    FIL file;
    const char *path="0:/RUZINA_TEST/Čitanje i seek 64GB.bin";
    FRESULT fr=f_open(&file,path,FA_READ);
    if(fr!=FR_OK) {
        printf("sd_fixture=OPEN_FAILED fresult=%u pass=%" PRIu32 "\n",(unsigned)fr,pass);
        return ESP_ERR_NOT_FOUND;
    }
    psa_hash_operation_t sha=PSA_HASH_OPERATION_INIT;
    esp_err_t err=ESP_FAIL;
    uint32_t offset=0;
    int64_t io_us=0, worst_io_us=0, seek_worst_us=0;
    uint8_t digest[32];
    size_t digest_size=0;
    if(f_size(&file)!=FIXTURE_SIZE || psa_crypto_init()!=PSA_SUCCESS ||
       psa_hash_setup(&sha,PSA_ALG_SHA_256)!=PSA_SUCCESS) goto finish;
    printf("sd_fixture_state=READING pass=%" PRIu32 " generation=%" PRIu32 " expected_bytes=%u\n",pass,generation,FIXTURE_SIZE);
    fflush(stdout);
    int64_t wall_start=esp_timer_get_time();
    while(offset<FIXTURE_SIZE) {
        UINT wanted=FIXTURE_SIZE-offset>4096 ? 4096 : FIXTURE_SIZE-offset, got=0;
        int64_t start=esp_timer_get_time();
        fr=f_read(&file,buffer,wanted,&got);
        int64_t elapsed=esp_timer_get_time()-start;
        io_us+=elapsed;
        if(elapsed>worst_io_us) worst_io_us=elapsed;
        if(fr!=FR_OK || got!=wanted || !matches(buffer,offset,got)) goto finish;
        if(psa_hash_update(&sha,buffer,got)!=PSA_SUCCESS) goto finish;
        offset+=got;
        /* Deliberate bounded yielding keeps the owner responsive and exposes
         * a repeatable open-file removal window. IO and wall time are separate. */
        if((offset & 16383u)==0) vTaskDelay(1);
    }
    if(psa_hash_finish(&sha,digest,sizeof(digest),&digest_size)!=PSA_SUCCESS ||
       digest_size!=32 || memcmp(digest,expected_sha,32)!=0) goto finish;
    UINT got=99;
    if(f_read(&file,buffer,4096,&got)!=FR_OK || got!=0) goto finish;
    const uint32_t seeks[]={FIXTURE_SIZE-513,0,131071,511,131072,4095,512,FIXTURE_SIZE-1,4096,FIXTURE_SIZE};
    for(unsigned i=0;i<sizeof(seeks)/sizeof(seeks[0]);++i) {
        UINT wanted=FIXTURE_SIZE-seeks[i]>513 ? 513 : FIXTURE_SIZE-seeks[i];
        int64_t start=esp_timer_get_time();
        if(f_lseek(&file,seeks[i])!=FR_OK || f_tell(&file)!=seeks[i]) goto finish;
        got=99;
        if(f_read(&file,buffer,wanted,&got)!=FR_OK || got!=wanted || !matches(buffer,seeks[i],got)) goto finish;
        int64_t elapsed=esp_timer_get_time()-start;
        if(elapsed>seek_worst_us) seek_worst_us=elapsed;
    }
    const int64_t wall_us=esp_timer_get_time()-wall_start;
    if(wall_us<io_us || io_us<=0 || worst_io_us<=0 || seek_worst_us<=0) {
        printf("sd_fixture_timing=INCONSISTENT\n");
        goto finish;
    }
    printf("sd_fixture=CONTENT_SHA256_AND_SEEKS_MATCH pass=%" PRIu32 " generation=%" PRIu32 " bytes=%u seek_cases=10 eof=MATCH sha256=",
           pass,generation,FIXTURE_SIZE);
    for(unsigned i=0;i<32;++i) printf("%02x",digest[i]);
    printf("\n");
    /* Keep separate timing records for independent start/end arithmetic.
     * Rechecking the complete original multi-value records found no printf
     * failure; their earlier inconsistent-time interpretation was incorrect. */
    printf("sd_fixture_io_us=%" PRId64 "\n",io_us);
    printf("sd_fixture_worst_read_us=%" PRId64 "\n",worst_io_us);
    printf("sd_fixture_worst_seek_read_us=%" PRId64 "\n",seek_worst_us);
    printf("sd_fixture_wall_start_us=%" PRId64 "\n",wall_start);
    printf("sd_fixture_wall_end_us=%" PRId64 "\n",wall_start+wall_us);
    printf("sd_fixture_wall_us=%" PRId64 "\n",wall_us);
    err=ESP_OK;
finish:
    psa_hash_abort(&sha);
    FRESULT close_result=f_close(&file);
    if(close_result!=FR_OK) err=ESP_FAIL;
    if(err!=ESP_OK) printf("sd_fixture=FAILED pass=%" PRIu32 " offset=%" PRIu32 " fresult=%u close_result=%u generation=%" PRIu32 "\n",
                            pass,offset,(unsigned)fr,(unsigned)close_result,generation);
    fflush(stdout);
    return err;
}
