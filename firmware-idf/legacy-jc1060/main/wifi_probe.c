/* C6 link probe: the deck's own Wi-Fi network and a status page, while the deck plays.
 *
 * ESP-Hosted carries Wi-Fi from the board's ESP32-C6 over SDIO. The probe starts an
 * access point, "Ruzina-XXXX", open and for the probe only, and an HTTP server on
 * 192.168.4.1:
 * - "/" is a page that shows both decks;
 * - "/status" is their state as JSON, which the page fetches every second.
 * The log carries counts and timings only: no addresses or names of the devices that join.
 *
 * ESP-Hosted starts its own tasks at priority 23 with no core affinity, above the USB
 * audio tasks (4 and 5). The probe lowers each one it knows to 2 as soon as it exists.
 * The HTTP server and lwIP run on the screen's core at priority 2. */
#include <inttypes.h>
#include <stdatomic.h>
#include <stdio.h>
#include <string.h>

#include "esp_event.h"
#include "esp_heap_caps.h"
#include "esp_hosted.h"
#include "esp_http_server.h"
#include "esp_mac.h"
#include "esp_netif.h"
#include "esp_timer.h"
#include "esp_wifi.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#define PROBE_PRIORITY 2
#define REPORT_PERIOD_MS 30000
#define AP_CHANNEL 6
#define AP_MAX_CLIENTS 4

extern void ruzina_legacy_engine_status(uint32_t *out);
extern int32_t ruzina_legacy_engine_track(uint32_t deck);

static const char *const hosted_tasks[] = {
    "rpc_rx", "rpc_tx", "rpc_supp_cb", "sdio_rx_buf", "sdio_read", "sdio_process_rx", "sdio_write",
};

static atomic_int clients;
static atomic_uint requests;
static atomic_uint request_max_us;

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

static void on_wifi_event(void *arg, esp_event_base_t base, int32_t id, void *data)
{
    (void)arg;
    (void)base;
    (void)data;
    if (id == WIFI_EVENT_AP_STACONNECTED) {
        printf("wifi_ap joined clients=%d\n", atomic_fetch_add(&clients, 1) + 1);
    } else if (id == WIFI_EVENT_AP_STADISCONNECTED) {
        printf("wifi_ap left clients=%d\n", atomic_fetch_sub(&clients, 1) - 1);
    }
}

static void note_request(int64_t start)
{
    const unsigned took = (unsigned)(esp_timer_get_time() - start);
    atomic_fetch_add(&requests, 1);
    unsigned seen = atomic_load(&request_max_us);
    while (took > seen && !atomic_compare_exchange_weak(&request_max_us, &seen, took)) {
    }
}

static const char page[] =
    "<!doctype html><html><head><meta charset=utf-8>"
    "<meta name=viewport content='width=device-width,initial-scale=1'><title>Ruzina</title>"
    "<style>body{font-family:sans-serif;background:#0d0f11;color:#eef0f2;margin:16px}"
    ".d{background:#12161a;border-radius:8px;padding:12px;margin:8px 0}"
    ".n{color:#38d6f0;font-weight:bold}.t{font-size:28px}.s{color:#9a9ea4}</style></head>"
    "<body><h2>Ruzina</h2><div class=d id=d1></div><div class=d id=d2></div>"
    "<p class=s id=u></p><script>"
    "function c(ms){var s=Math.floor(ms/1000);return Math.floor(s/60)+':'+('0'+s%60).slice(-2)}"
    "function d(e,n,x){e.innerHTML='<span class=n>DECK '+n+'</span> '+(x.loaded?"
    "(x.playing?'&#9654; PLAY':'&#10074;&#10074; PAUSE')+' &middot; track '+(x.track+1)+"
    "'<div class=t>'+c(x.ms)+'</div>':'<span class=s>empty</span>')}"
    "function p(){fetch('/status').then(r=>r.json()).then(j=>{"
    "d(document.getElementById('d1'),1,j.decks[0]);d(document.getElementById('d2'),2,j.decks[1]);"
    "document.getElementById('u').textContent='uptime '+c(j.uptime_ms)}).catch(()=>{})}"
    "p();setInterval(p,1000)</script></body></html>";

static esp_err_t serve_page(httpd_req_t *req)
{
    const int64_t start = esp_timer_get_time();
    httpd_resp_set_type(req, "text/html");
    esp_err_t err = httpd_resp_send(req, page, sizeof page - 1);
    note_request(start);
    return err;
}

static esp_err_t serve_status(httpd_req_t *req)
{
    const int64_t start = esp_timer_get_time();
    uint32_t status[4] = {0};
    ruzina_legacy_engine_status(status);
    char body[256];
    int len = snprintf(body, sizeof body,
                       "{\"uptime_ms\":%" PRId64 ",\"decks\":["
                       "{\"ms\":%" PRIu32 ",\"loaded\":%s,\"playing\":%s,\"track\":%" PRId32 "},"
                       "{\"ms\":%" PRIu32 ",\"loaded\":%s,\"playing\":%s,\"track\":%" PRId32 "}]}",
                       esp_timer_get_time() / 1000,
                       status[0], (status[1] & 1) ? "true" : "false", (status[1] & 2) ? "true" : "false",
                       ruzina_legacy_engine_track(1),
                       status[2], (status[3] & 1) ? "true" : "false", (status[3] & 2) ? "true" : "false",
                       ruzina_legacy_engine_track(2));
    httpd_resp_set_type(req, "application/json");
    httpd_resp_set_hdr(req, "Cache-Control", "no-store");
    esp_err_t err = httpd_resp_send(req, body, len);
    note_request(start);
    return err;
}

/* Browsers ask for an icon: no content, rather than a 404 warning in the log. */
static esp_err_t serve_no_icon(httpd_req_t *req)
{
    httpd_resp_set_status(req, "204 No Content");
    return httpd_resp_send(req, NULL, 0);
}

static esp_err_t start_server(void)
{
    httpd_config_t config = HTTPD_DEFAULT_CONFIG();
    config.task_priority = PROBE_PRIORITY;
    config.core_id = 1;
    config.max_open_sockets = 4;
    config.lru_purge_enable = true;
    httpd_handle_t server = NULL;
    esp_err_t err = httpd_start(&server, &config);
    if (err != ESP_OK) return err;
    const httpd_uri_t root = {.uri = "/", .method = HTTP_GET, .handler = serve_page};
    const httpd_uri_t status = {.uri = "/status", .method = HTTP_GET, .handler = serve_status};
    const httpd_uri_t icon = {.uri = "/favicon.ico", .method = HTTP_GET, .handler = serve_no_icon};
    httpd_register_uri_handler(server, &root);
    httpd_register_uri_handler(server, &status);
    httpd_register_uri_handler(server, &icon);
    return ESP_OK;
}

static esp_err_t start_access_point(void)
{
    esp_netif_create_default_wifi_ap();
    wifi_init_config_t init = WIFI_INIT_CONFIG_DEFAULT();
    esp_err_t err = esp_wifi_init(&init);
    lower_hosted_tasks("wifi_init");
    if (err != ESP_OK) return err;
    esp_event_handler_register(WIFI_EVENT, ESP_EVENT_ANY_ID, on_wifi_event, NULL);
    /* The P4 has no Wi-Fi MAC of its own: the access point's address is the C6's. */
    uint8_t mac[6] = {0};
    esp_wifi_get_mac(WIFI_IF_AP, mac);
    wifi_config_t config = {0};
    int len = snprintf((char *)config.ap.ssid, sizeof config.ap.ssid, "Ruzina-%02X%02X", mac[4], mac[5]);
    config.ap.ssid_len = (uint8_t)len;
    config.ap.channel = AP_CHANNEL;
    config.ap.max_connection = AP_MAX_CLIENTS;
    config.ap.authmode = WIFI_AUTH_OPEN;
    err = esp_wifi_set_mode(WIFI_MODE_AP);
    if (err == ESP_OK) err = esp_wifi_set_config(WIFI_IF_AP, &config);
    if (err == ESP_OK) err = esp_wifi_start();
    printf("wifi_ap start=%s ssid=%s channel=%d\n", esp_err_to_name(err), (const char *)config.ap.ssid, AP_CHANNEL);
    return err;
}

static void probe_task(void *unused)
{
    (void)unused;
    lower_hosted_tasks("start");
    ESP_ERROR_CHECK_WITHOUT_ABORT(esp_netif_init());
    ESP_ERROR_CHECK_WITHOUT_ABORT(esp_event_loop_create_default());
    esp_err_t err = start_access_point();
    if (err == ESP_OK) err = start_server();
    printf("wifi_probe http=%s\n", esp_err_to_name(err));
    /* By now the link is up: the C6's firmware version. */
    esp_hosted_coprocessor_fwver_t version = {0};
    if (esp_hosted_get_coprocessor_fwversion(&version) == ESP_OK) {
        printf("wifi_probe c6_firmware=%" PRIu32 ".%" PRIu32 ".%" PRIu32 "\n",
               version.major1, version.minor1, version.patch1);
    }
    if (err != ESP_OK) {
        vTaskDelete(NULL);
        return;
    }
    for (;;) {
        vTaskDelay(pdMS_TO_TICKS(REPORT_PERIOD_MS));
        lower_hosted_tasks("report");
        printf("wifi_ap clients=%d requests=%u request_max_us=%u internal_free=%u\n",
               atomic_load(&clients), atomic_load(&requests), atomic_exchange(&request_max_us, 0),
               (unsigned)heap_caps_get_free_size(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT));
    }
}

esp_err_t ruzina_wifi_probe_start(void)
{
    /* The screen's core, below the decoders and the screen. */
    return xTaskCreatePinnedToCore(probe_task, "wifi_probe", 6144, NULL, PROBE_PRIORITY, NULL, 1) == pdPASS
               ? ESP_OK
               : ESP_ERR_NO_MEM;
}
