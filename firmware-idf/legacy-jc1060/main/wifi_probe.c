/* C6 link probe: reads the ESP32-C6's ESP-Hosted firmware over SDIO, starts Wi-Fi as a
 * station and scans now and then, while the deck plays. It never joins a network and
 * prints no network names: only counts, signal strength and channels.
 *
 * ESP-Hosted starts its own tasks at priority 23 with no core affinity, above the USB
 * audio tasks (4 and 5). This probe lowers each one it knows to 2 as soon as it exists,
 * and reports any it could not find. */
#include <inttypes.h>
#include <stdio.h>
#include <string.h>

#include "esp_event.h"
#include "esp_hosted.h"
#include "esp_hosted_misc.h"
#include "esp_netif.h"
#include "esp_timer.h"
#include "esp_wifi.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#define PROBE_PRIORITY 2
#define SCAN_PERIOD_MS 30000
#define SCAN_RECORDS 24

static const char *const hosted_tasks[] = {
    "rpc_rx", "rpc_tx", "rpc_supp_cb", "sdio_rx_buf", "sdio_read", "sdio_process_rx", "sdio_write",
};

static void lower_hosted_tasks(const char *when)
{
    unsigned lowered = 0, missing = 0;
    for (size_t i = 0; i < sizeof hosted_tasks / sizeof hosted_tasks[0]; ++i) {
        TaskHandle_t task = xTaskGetHandle(hosted_tasks[i]);
        if (task == NULL) {
            ++missing;
            continue;
        }
        if (uxTaskPriorityGet(task) != PROBE_PRIORITY) {
            vTaskPrioritySet(task, PROBE_PRIORITY);
        }
        ++lowered;
    }
    printf("wifi_probe tasks when=%s lowered=%u missing=%u\n", when, lowered, missing);
}

static void scan_once(void)
{
    static wifi_ap_record_t records[SCAN_RECORDS];
    const int64_t start = esp_timer_get_time();
    esp_err_t err = esp_wifi_scan_start(NULL, true);
    const int64_t took = esp_timer_get_time() - start;
    if (err != ESP_OK) {
        printf("wifi_scan error=%s us=%" PRId64 "\n", esp_err_to_name(err), took);
        return;
    }
    uint16_t found = 0;
    esp_wifi_scan_get_ap_num(&found);
    uint16_t count = SCAN_RECORDS;
    err = esp_wifi_scan_get_ap_records(&count, records);
    if (err != ESP_OK) {
        printf("wifi_scan records_error=%s found=%u\n", esp_err_to_name(err), found);
        return;
    }
    /* ESP-Hosted's wrapper copies the records but leaves `count` at the capacity
     * (rpc_wrap.c): only the first `found` hold networks. */
    if (count > found) count = found;
    int best = -127;
    uint32_t channels = 0;
    for (uint16_t i = 0; i < count; ++i) {
        if (records[i].rssi > best) best = records[i].rssi;
        if (records[i].primary < 32) channels |= 1u << records[i].primary;
    }
    printf("wifi_scan found=%u best_rssi=%d channels=0x%08" PRIx32 " us=%" PRId64 "\n",
           found, count ? best : 0, channels, took);
}

static void probe_task(void *unused)
{
    (void)unused;
    lower_hosted_tasks("start");
    esp_hosted_coprocessor_fwver_t version = {0};
    esp_err_t err = esp_hosted_get_coprocessor_fwversion(&version);
    if (err == ESP_OK) {
        printf("wifi_probe c6_firmware=%" PRIu32 ".%" PRIu32 ".%" PRIu32 "\n",
               version.major1, version.minor1, version.patch1);
    } else {
        printf("wifi_probe c6_firmware_error=%s\n", esp_err_to_name(err));
    }
    esp_hosted_app_desc_t app = {0};
    if (esp_hosted_get_coprocessor_app_desc(&app) == ESP_OK) {
        printf("wifi_probe c6_app project=%.32s version=%.32s idf=%.32s\n",
               app.project_name, app.version, app.idf_ver);
    }
    ESP_ERROR_CHECK_WITHOUT_ABORT(esp_netif_init());
    ESP_ERROR_CHECK_WITHOUT_ABORT(esp_event_loop_create_default());
    esp_netif_create_default_wifi_sta();
    wifi_init_config_t config = WIFI_INIT_CONFIG_DEFAULT();
    err = esp_wifi_init(&config);
    lower_hosted_tasks("wifi_init");
    if (err == ESP_OK) err = esp_wifi_set_mode(WIFI_MODE_STA);
    if (err == ESP_OK) err = esp_wifi_start();
    printf("wifi_probe start=%s\n", esp_err_to_name(err));
    if (err != ESP_OK) {
        vTaskDelete(NULL);
        return;
    }
    for (;;) {
        scan_once();
        lower_hosted_tasks("scan");
        vTaskDelay(pdMS_TO_TICKS(SCAN_PERIOD_MS));
    }
}

esp_err_t ruzina_wifi_probe_start(void)
{
    /* The screen's core, below the decoders and the screen. */
    return xTaskCreatePinnedToCore(probe_task, "wifi_probe", 6144, NULL, PROBE_PRIORITY, NULL, 1) == pdPASS
               ? ESP_OK
               : ESP_ERR_NO_MEM;
}
