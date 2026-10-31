#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include "sdkconfig.h"
#include "esp_chip_info.h"
#if !CONFIG_APP_BUILD_TYPE_PURE_RAM_APP
#include "esp_flash.h"
#endif
#include "esp_idf_version.h"
#include "esp_system.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#if ESP_IDF_VERSION != ESP_IDF_VERSION_VAL(6, 1, 0)
#error "This compile experiment is pinned to ESP-IDF 6.1"
#endif

/* Scalar ABI only: no Rust layout, allocations, pointers or exceptions cross it. */
extern int16_t ruzina_legacy_pitch_centipercent(uint16_t raw);

#if !CONFIG_APP_BUILD_TYPE_PURE_RAM_APP
static void restart_console(void *unused)
{
    (void)unused;
    for (;;) {
        /* An explicit uppercase R from the operator requests an app restart.
         * This task may block on console input; heartbeat runs independently. */
        const int command = getchar();
        if (command == EOF) {
            clearerr(stdin);
        }
        if (command == 'R') {
            printf("restart_request=OPERATOR_CONSOLE_R\n");
            fflush(stdout);
            vTaskDelay(pdMS_TO_TICKS(100));
            esp_restart();
        }
        vTaskDelay(pdMS_TO_TICKS(20));
    }
}
#endif

void app_main(void)
{
    esp_chip_info_t chip;
    esp_chip_info(&chip);
    printf("RUZINA_LEGACY_IDF_PROBE_V1\nchip_revision=%u\n", (unsigned)chip.revision);
    printf("reset_reason=%u\n", (unsigned)esp_reset_reason());
    if (chip.model != CHIP_ESP32P4 || chip.revision != 103) {
        printf("result=WRONG_UNIT_REVISION\nreadiness=NOT_QUALIFIED\n");
        return;
    }
#if !CONFIG_APP_BUILD_TYPE_PURE_RAM_APP
    uint32_t flash_bytes = 0;
    if (esp_flash_get_size(NULL, &flash_bytes) != ESP_OK || flash_bytes != 16777216) {
        printf("result=FLASH_GEOMETRY_MISMATCH\nreadiness=NOT_QUALIFIED\n");
        return;
    }
#else
    printf("execution=PURE_RAM\nflash_bytes=NOT_MEASURED\n");
#endif
    const uint16_t raw[] = {0, 8192, 16383, UINT16_MAX};
    const int16_t expected[] = {1000, 0, -999, -999};
    for (unsigned i = 0; i < sizeof(raw) / sizeof(raw[0]); ++i) {
        const int16_t value = ruzina_legacy_pitch_centipercent(raw[i]);
        printf("pitch_raw=%u pitch_centipercent=%d\n", raw[i], value);
        if (value != expected[i]) {
            printf("result=RUST_ABI_FAILURE\nreadiness=NOT_QUALIFIED\n");
            return;
        }
    }
#if !CONFIG_APP_BUILD_TYPE_PURE_RAM_APP
    printf("flash_bytes=%" PRIu32 "\n", flash_bytes);
#endif
    printf("result=SCALAR_RUST_CALLS_MATCH\nreadiness=NOT_QUALIFIED\n");
#if CONFIG_RUZINA_MEMORY_TASK
    extern bool ruzina_memory_task_probe(void);
    if (!ruzina_memory_task_probe()) {
        printf("s5_result=FAIL\nreadiness=NOT_QUALIFIED\n");
        fflush(stdout);
        return;
    }
    printf("s5_result=BOUNDED_MEMORY_FPU_EQ_SMOKE_MATCH\nreadiness=NOT_QUALIFIED\n");
#endif
#if CONFIG_RUZINA_DISPLAY_TOUCH
    extern esp_err_t ruzina_display_touch_start(void);
    if (ruzina_display_touch_start() != ESP_OK) {
        printf("s6_result=INIT_FAILED\nreadiness=NOT_QUALIFIED\n");
    }
#endif
#if CONFIG_RUZINA_SD_READONLY
    extern esp_err_t ruzina_sd_readonly_start(void);
    if (ruzina_sd_readonly_start() != ESP_OK) {
        printf("sd_result=INIT_FAILED\nreadiness=NOT_QUALIFIED\n");
    }
#endif
#if CONFIG_RUZINA_LOAD_DISPLAY
    /* Panel first, so the screen shows the mount; the load runs without it. */
    extern esp_err_t ruzina_load_display_start(void);
    if (ruzina_load_display_start() != ESP_OK) {
        printf("load_display=INIT_FAILED\n");
    }
#endif
#if CONFIG_RUZINA_SD_RUST_BRIDGE
    extern esp_err_t ruzina_sd_bridge_start(void);
    if (ruzina_sd_bridge_start() != ESP_OK) {
        printf("rust_sd_bridge=INIT_FAILED\nreadiness=NOT_QUALIFIED\n");
    }
#endif
#if !CONFIG_APP_BUILD_TYPE_PURE_RAM_APP
    if (xTaskCreate(restart_console, "probe_restart", 2048, NULL, 1, NULL) != pdPASS) {
        printf("result=CONSOLE_TASK_CREATE_FAILED\n");
        fflush(stdout);
        return;
    }
    printf("execution=FLASH_BACKED\nrestart_command=UPPERCASE_R\n");
#else
    printf("restart_command=DISABLED_IN_PURE_RAM\n");
#endif
    fflush(stdout);
    uint32_t heartbeat = 0;
    for (;;) {
        vTaskDelay(pdMS_TO_TICKS(1000));
        printf("heartbeat=%" PRIu32 " uptime_us=%" PRId64 "\n",
               ++heartbeat, esp_timer_get_time());
        fflush(stdout);
    }
}
