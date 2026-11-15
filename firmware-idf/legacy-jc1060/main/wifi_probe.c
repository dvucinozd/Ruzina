/* The deck's own Wi-Fi and the Web Remote, over the board's ESP32-C6.
 *
 * ESP-Hosted carries Wi-Fi from the C6 over SDIO. The deck starts a WPA2 access point,
 * "Ruzina-XXXX", with the password CONFIG_RUZINA_WIFI_PASSWORD (the owner's choice), shown
 * on the Settings page (ruzina_wifi_credentials). On 192.168.4.1 a small
 * server takes one connection at a time and hands each request to ruzina-web
 * (ruzina_web_handle): the Web Remote page, its status and its controls.
 * The log carries counts and timings only: no addresses or names of the devices that
 * join, and never the password.
 *
 * ESP-Hosted starts its own tasks at priority 23 with no core affinity, above the USB
 * audio tasks (4 and 5). The probe lowers each one it knows to 2 as soon as it exists.
 * The server and lwIP run on the screen's core at priority 2. */
#include <errno.h>
#include <inttypes.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>

#include "esp_event.h"
#include "esp_heap_caps.h"
#include "esp_hosted.h"
#include "esp_netif.h"
#include "esp_random.h"
#include "esp_timer.h"
#include "esp_wifi.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "lwip/sockets.h"

#define PROBE_PRIORITY 2
#define REPORT_PERIOD_MS 30000
#define AP_CHANNEL 6
#define AP_MAX_CLIENTS 4
#define REQUEST_MAX 2048

extern size_t ruzina_web_handle(uint32_t boot, const uint8_t *request, size_t request_len, uint8_t *out,
                                size_t out_len);
extern size_t ruzina_web_largest_asset(void);

static const char *const hosted_tasks[] = {
    "rpc_rx", "rpc_tx", "rpc_supp_cb", "sdio_rx_buf", "sdio_read", "sdio_process_rx", "sdio_write",
};

static char ap_ssid[33];
static const char ap_password[] = CONFIG_RUZINA_WIFI_PASSWORD;
_Static_assert(sizeof ap_password <= sizeof(((wifi_ap_config_t *)0)->password),
               "CONFIG_RUZINA_WIFI_PASSWORD is longer than WPA2 allows");
static atomic_bool ap_ready;
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

/* The Settings page's network and password; false until the access point runs. */
bool ruzina_wifi_credentials(char *ssid, size_t ssid_len, char *password, size_t password_len)
{
    if (!atomic_load(&ap_ready) || ssid_len == 0 || password_len == 0) return false;
    snprintf(ssid, ssid_len, "%s", ap_ssid);
    snprintf(password, password_len, "%s", ap_password);
    return true;
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
    snprintf(ap_ssid, sizeof ap_ssid, "Ruzina-%02X%02X", mac[4], mac[5]);
    wifi_config_t config = {0};
    memcpy(config.ap.ssid, ap_ssid, strlen(ap_ssid));
    config.ap.ssid_len = (uint8_t)strlen(ap_ssid);
    memcpy(config.ap.password, ap_password, strlen(ap_password));
    config.ap.channel = AP_CHANNEL;
    config.ap.max_connection = AP_MAX_CLIENTS;
    /* WPA2 needs 8 to 63 characters; anything else leaves the network open. */
    const size_t password_len = strlen(ap_password);
    config.ap.authmode = password_len >= 8 && password_len <= 63 ? WIFI_AUTH_WPA2_PSK : WIFI_AUTH_OPEN;
    config.ap.pmf_cfg.capable = true;
    err = esp_wifi_set_mode(WIFI_MODE_AP);
    if (err == ESP_OK) err = esp_wifi_set_config(WIFI_IF_AP, &config);
    if (err == ESP_OK) err = esp_wifi_start();
    printf("wifi_ap start=%s ssid=%s channel=%d auth=%s\n", esp_err_to_name(err), ap_ssid, AP_CHANNEL,
           config.ap.authmode == WIFI_AUTH_WPA2_PSK ? "wpa2" : "open");
    if (err == ESP_OK) atomic_store(&ap_ready, true);
    return err;
}

/* One request: the head, then a body up to Content-Length, both within REQUEST_MAX. */
static size_t read_request(int sock, uint8_t *buffer)
{
    size_t len = 0;
    size_t want = 0;
    while (len < REQUEST_MAX) {
        int got = recv(sock, buffer + len, REQUEST_MAX - len, 0);
        if (got <= 0) return want && len >= want ? len : 0;
        len += (size_t)got;
        if (want == 0) {
            const uint8_t *end = memmem(buffer, len, "\r\n\r\n", 4);
            if (end == NULL) continue;
            size_t head = (size_t)(end - buffer) + 4;
            size_t body = 0;
            const char *field = strcasestr((const char *)buffer, "\r\nContent-Length:");
            if (field != NULL && (const uint8_t *)field < end) body = strtoul(field + 17, NULL, 10);
            want = head + body;
            if (want > REQUEST_MAX) return 0;
        }
        if (len >= want) return len;
    }
    return 0;
}

static void serve(int listener, uint8_t *request, uint8_t *response, size_t response_cap, uint32_t boot)
{
    struct sockaddr_storage peer;
    socklen_t peer_len = sizeof peer;
    int sock = accept(listener, (struct sockaddr *)&peer, &peer_len);
    if (sock < 0) return;
    const struct timeval timeout = {.tv_sec = 2, .tv_usec = 0};
    setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof timeout);
    setsockopt(sock, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof timeout);
    const int64_t start = esp_timer_get_time();
    size_t len = read_request(sock, request);
    if (len > 0) {
        size_t out = ruzina_web_handle(boot, request, len, response, response_cap);
        for (size_t sent = 0; sent < out;) {
            int n = send(sock, response + sent, out - sent, 0);
            if (n <= 0) break;
            sent += (size_t)n;
        }
        const unsigned took = (unsigned)(esp_timer_get_time() - start);
        atomic_fetch_add(&requests, 1);
        if (took > atomic_load(&request_max_us)) atomic_store(&request_max_us, took);
    }
    shutdown(sock, SHUT_RDWR);
    close(sock);
}

static void server_task(void *unused)
{
    (void)unused;
    const size_t response_cap = ruzina_web_largest_asset() + 1024;
    uint8_t *request = heap_caps_malloc(REQUEST_MAX, MALLOC_CAP_SPIRAM);
    uint8_t *response = heap_caps_malloc(response_cap, MALLOC_CAP_SPIRAM);
    int listener = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    struct sockaddr_in address = {.sin_family = AF_INET, .sin_port = htons(80), .sin_addr.s_addr = htonl(INADDR_ANY)};
    const int yes = 1;
    setsockopt(listener, SOL_SOCKET, SO_REUSEADDR, &yes, sizeof yes);
    bool ok = request && response && listener >= 0 &&
              bind(listener, (struct sockaddr *)&address, sizeof address) == 0 && listen(listener, 4) == 0;
    printf("web_remote listen=%s response_cap=%u\n", ok ? "ok" : "failed", (unsigned)response_cap);
    if (!ok) {
        vTaskDelete(NULL);
        return;
    }
    /* Differs on each start, so the page notices a reboot. */
    const uint32_t boot = esp_random();
    for (;;) serve(listener, request, response, response_cap, boot);
}

static void probe_task(void *unused)
{
    (void)unused;
    lower_hosted_tasks("start");
    ESP_ERROR_CHECK_WITHOUT_ABORT(esp_netif_init());
    ESP_ERROR_CHECK_WITHOUT_ABORT(esp_event_loop_create_default());
    esp_err_t err = start_access_point();
    if (err == ESP_OK &&
        xTaskCreatePinnedToCore(server_task, "web_remote", 6144, NULL, PROBE_PRIORITY, NULL, 1) != pdPASS) {
        err = ESP_ERR_NO_MEM;
    }
    /* By now the link is up: the C6's firmware version. */
    esp_hosted_coprocessor_fwver_t version = {0};
    if (esp_hosted_get_coprocessor_fwversion(&version) == ESP_OK) {
        printf("wifi_probe c6_firmware=%" PRIu32 ".%" PRIu32 ".%" PRIu32 "\n",
               version.major1, version.minor1, version.patch1);
    }
    if (err != ESP_OK) {
        printf("wifi_probe error=%s\n", esp_err_to_name(err));
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
