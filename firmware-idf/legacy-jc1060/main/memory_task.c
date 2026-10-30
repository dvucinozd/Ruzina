#include <inttypes.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include "esp_heap_caps.h"
#include "esp_memory_utils.h"
#include "esp_psram.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "freertos/semphr.h"

extern float ruzina_legacy_fp_hold(float seed, uint32_t count, _Atomic uint32_t *active);
extern int32_t ruzina_legacy_eq_block(const int16_t *input, int16_t *output, size_t frames);
static _Atomic uint32_t active[2];
static SemaphoreHandle_t done;
static struct {
    uint32_t id, mismatches, overlaps, stack_free;
    int core;
} workers[2];

static void fp_worker(void *arg)
{
    const unsigned id = *(uint32_t *)arg;
    for (unsigned k = 0; k < 32; ++k) {
        if (atomic_load_explicit(&active[1 - id], memory_order_acquire)) {
            workers[id].overlaps++;
        }
        const float seed = (float)(16 + id * 128 + k);
        const float got = ruzina_legacy_fp_hold(seed, 4000000, &active[id]);
        /* All integers through 4,000,175 are exactly representable in f32. */
        if (got != seed + 4000000.0f) {
            workers[id].mismatches++;
        }
        vTaskDelay(1);
    }
    workers[id].core = xPortGetCoreID();
    workers[id].stack_free = uxTaskGetStackHighWaterMark(NULL);
    xSemaphoreGive(done);
    vTaskDelete(NULL);
}

static bool fp_campaign(bool dual)
{
    done = xSemaphoreCreateCounting(2, 0);
    if (!done) return false;
    for (unsigned id = 0; id < 2; ++id) {
        workers[id].id = id;
        workers[id].mismatches = workers[id].overlaps = 0;
        atomic_store(&active[id], 0);
    }
    unsigned created = 0;
    for (unsigned id = 0; id < 2; ++id) {
        if (xTaskCreatePinnedToCore(fp_worker, "s5_fp", 4096, &workers[id].id, 3,
                                    NULL, dual ? (int)id : 0) != pdPASS) break;
        created++;
    }
    /* Bound runtime with watchdogs left enabled. No semaphore deletion while
     * a timed-out worker may still use it: failure terminates this campaign. */
    for (unsigned id = 0; id < created; ++id) {
        if (xSemaphoreTake(done, pdMS_TO_TICKS(30000)) != pdTRUE) {
            printf("fpu_timeout=1\n");
            return false;
        }
    }
    vSemaphoreDelete(done);
    uint32_t mismatch = 0, overlap = 0;
    for (unsigned id = 0; id < created; ++id) {
        printf("fpu_mode=%s worker=%u core=%d mismatches=%" PRIu32 " live_other=%" PRIu32 " stack_free_bytes=%" PRIu32 "\n",
               dual ? "dual" : "same-core", id, workers[id].core,
               workers[id].mismatches, workers[id].overlaps, workers[id].stack_free);
        mismatch += workers[id].mismatches;
        overlap += workers[id].overlaps;
        if (workers[id].core != (dual ? (int)id : 0)) mismatch++;
    }
    return created == 2 && mismatch == 0 && overlap > 0;
}

bool ruzina_memory_task_probe(void)
{
    const uint32_t caps = MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT;
    const size_t capacity = esp_psram_get_size();
    printf("S5_MEMORY_TASK_V1 psram_bytes=%u speed_mhz=80 cpu_mhz=360\n", (unsigned)capacity);
    if (!esp_psram_is_initialized() || capacity == 0 || !heap_caps_check_integrity_all(true)) return false;
    printf("heap_internal_free=%u heap_external_free=%u\n",
           (unsigned)heap_caps_get_free_size(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT),
           (unsigned)heap_caps_get_free_size(caps));
    volatile uint32_t *scratch = heap_caps_malloc(65536 + 8, caps);
    uint32_t *internal = heap_caps_malloc(4096, MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT);
    if (!scratch || !internal) {
        heap_caps_free((void *)scratch); heap_caps_free(internal); return false;
    }
    bool ok = esp_ptr_external_ram((const void *)scratch) && esp_ptr_internal(internal);
    scratch[0] = 0x13579bdf; scratch[16385] = 0x2468ace0;
    uint32_t errors = 0;
    for (uint32_t pass = 0; pass < 4; ++pass) {
        for (uint32_t i = 0; i < 16384; ++i) {
            uint32_t value = pass == 0 ? 0x55555555 : pass == 1 ? 0xaaaaaaaa : (i * 4) ^ 0xa5a55a5a;
            scratch[i + 1] = pass == 3 ? ~value : value;
        }
        for (uint32_t i = 0; i < 16384; ++i) {
            uint32_t value = pass == 0 ? 0x55555555 : pass == 1 ? 0xaaaaaaaa : (i * 4) ^ 0xa5a55a5a;
            if (scratch[i + 1] != (pass == 3 ? ~value : value)) errors++;
        }
    }
    for (unsigned i = 0; i < 1024; ++i) internal[i] = i ^ 0x5a5a;
    for (unsigned i = 0; i < 1024; ++i) if (internal[i] != (i ^ 0x5a5a)) errors++;
    ok &= errors == 0 && scratch[0] == 0x13579bdf && scratch[16385] == 0x2468ace0;
    void *impossible = heap_caps_malloc(capacity + 1, caps);
    const bool allocation_failure = impossible == NULL;
    heap_caps_free(impossible);
    printf("scratch_bytes=65536 pattern_passes=4 mismatches=%" PRIu32 " canaries=%s allocation_failure=%s\n",
           errors, ok ? "MATCH" : "FAIL", allocation_failure ? "EXPECTED_NULL" : "UNEXPECTED_SUCCESS");
    heap_caps_free((void *)scratch); heap_caps_free(internal);
    if (!ok || !allocation_failure || !heap_caps_check_integrity_all(true)) return false;
    if (!fp_campaign(false) || !fp_campaign(true)) return false;

    /* C allocates and frees both buffers. Rust borrows disjoint arrays only
     * during each synchronous call. No DMA or shared layout is involved. */
    int16_t *input = heap_caps_malloc(1024, MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT);
    int16_t *output = heap_caps_malloc(1024, MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT);
    if (!input || !output) { heap_caps_free(input); heap_caps_free(output); return false; }
    for (unsigned i = 0; i < 512; ++i) input[i] = (int16_t)((i * 53u) % 10000u - 5000);
    ok = ruzina_legacy_eq_block(NULL, output, 256) == -1
         && ruzina_legacy_eq_block(input, output, 257) == -1
         && ruzina_legacy_eq_block(input, input, 256) == -1
         && ruzina_legacy_eq_block(input, (int16_t *)((uint8_t *)output + 1), 256) == -1;
    int64_t worst = 0;
    uint32_t checksum = 0;
    for (unsigned block = 0; block < 64; ++block) {
        int64_t start = esp_timer_get_time();
        int32_t status = ruzina_legacy_eq_block(input, output, 256);
        int64_t elapsed = esp_timer_get_time() - start;
        if (elapsed > worst) worst = elapsed;
        if (status != 0) ok = false;
        for (unsigned i = 0; i < 512; ++i) checksum = checksum * 31u + (uint16_t)output[i];
        vTaskDelay(1);
    }
    printf("eq_frames=256 rate_hz=44100 blocks=64 deadline_us=5804 worst_us=%" PRId64 " checksum=%" PRIu32 " buffer_rejections=%s\n",
           worst, checksum, ok ? "MATCH" : "FAIL");
    heap_caps_free(input); heap_caps_free(output);
    printf("heap_internal_min=%u heap_external_min=%u main_stack_free_bytes=%u\n",
           (unsigned)heap_caps_get_minimum_free_size(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT),
           (unsigned)heap_caps_get_minimum_free_size(caps), (unsigned)uxTaskGetStackHighWaterMark(NULL));
    return ok && worst < 5804 && heap_caps_check_integrity_all(true);
}
