/* The deck's own Wi-Fi and the Web Remote, over the board's ESP32-C6.
 *
 * ESP-Hosted carries Wi-Fi from the C6 over SDIO. The deck starts a WPA2 access point,
 * "Ruzina-XXXX", with the password CONFIG_RUZINA_WIFI_PASSWORD (the owner's choice), shown
 * on the Settings page (ruzina_wifi_credentials). A password set from the Web Remote's
 * SETUP page replaces it (ruzina_wifi_change_password); it is kept in NVS, and the deck's
 * Settings page can put the default back. On 192.168.4.1 a small
 * server serves up to four connections at once (web_client.h) and hands each request to
 * ruzina-web (ruzina_web_handle): the Web Remote page, its status and its controls. A
 * firmware bundle streams through ruzina-web into the update slot (ota_slot.c); once it is
 * in and checked, the deck restarts into it, and keeps it when the server listens again.
 * The log carries counts and timings only: no addresses or names of the devices that
 * join, and never the password.
 *
 * ESP-Hosted starts its own tasks at priority 23 with no core affinity, above the USB
 * audio tasks (4 and 5). Each one it knows is lowered to 2 as soon as it exists.
 * The server and lwIP run on the screen's core at priority 2. */
#include <ctype.h>
#include <errno.h>
#include <inttypes.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "esp_event.h"
#include "esp_heap_caps.h"
#include "esp_hosted.h"
#include "esp_netif.h"
#include "esp_random.h"
#include "esp_system.h"
#include "esp_timer.h"
#include "esp_wifi.h"
#include "nvs.h"
#include "nvs_flash.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "lwip/sockets.h"

#define LOW_PRIORITY 2
#define REPORT_PERIOD_MS 30000
#define AP_CHANNEL 6
#define AP_MAX_CLIENTS 4
#define REQUEST_MAX 2048

extern size_t ruzina_web_handle(uint32_t boot, const uint8_t *request, size_t request_len, uint8_t *out,
                                size_t out_len);
extern size_t ruzina_web_largest_asset(void);
extern void ruzina_web_set_name(const uint8_t *name, size_t len);
extern int ruzina_web_stream_open(uint32_t boot, const uint8_t *head, size_t len);
extern int ruzina_web_stream_feed(const uint8_t *bytes, size_t len);
extern void ruzina_web_stream_cancel(void);
extern size_t ruzina_web_stream_finish(int status, uint8_t *out, size_t out_len);
extern bool ruzina_web_restart_due(void);
extern void ruzina_ota_report(void);
extern void ruzina_ota_settle(bool healthy);

/* Called from ruzina_web_handle: the web task waits a moment for the USB task to answer a
 * load or a seek. At least one tick, so the USB task gets its turn. */
void ruzina_web_wait(uint32_t ms)
{
    const TickType_t ticks = pdMS_TO_TICKS(ms);
    vTaskDelay(ticks ? ticks : 1);
}

static const char *const hosted_tasks[] = {
    "rpc_rx", "rpc_tx", "rpc_supp_cb", "sdio_rx_buf", "sdio_read", "sdio_process_rx", "sdio_write",
};

static char ap_ssid[33];
static const char default_password[] = CONFIG_RUZINA_WIFI_PASSWORD;
_Static_assert(sizeof default_password <= sizeof(((wifi_ap_config_t *)0)->password),
               "CONFIG_RUZINA_WIFI_PASSWORD is longer than WPA2 allows");
/* The password in force: the default, or one the owner set (kept in NVS). Read and written
 * under ap_lock: the Settings page reads it on the screen's task, a change comes on the
 * app main task. */
static char ap_password[65] = CONFIG_RUZINA_WIFI_PASSWORD;
static portMUX_TYPE ap_lock = portMUX_INITIALIZER_UNLOCKED;
#define NVS_NAMESPACE "ruzina"
#define NVS_PASSWORD_KEY "ap_password"
static atomic_bool ap_ready;
static atomic_int clients;
static atomic_uint requests;
static atomic_uint request_max_us;
/* The web server's state, for the Wi-Fi task's report. */
enum { SERVER_STARTING, SERVER_LISTENING, SERVER_FAILED };
static atomic_int server_state;

static void lower_hosted_tasks(const char *when)
{
    unsigned lowered = 0, missing = 0;
    for (size_t i = 0; i < sizeof hosted_tasks / sizeof hosted_tasks[0]; ++i) {
        TaskHandle_t task = xTaskGetHandle(hosted_tasks[i]);
        if (task == NULL) {
            ++missing;
            continue;
        }
        if (uxTaskPriorityGet(task) != LOW_PRIORITY) {
            vTaskPrioritySet(task, LOW_PRIORITY);
        }
        ++lowered;
    }
    printf("wifi tasks when=%s lowered=%u missing=%u\n", when, lowered, missing);
}

/* The Settings page's network and password; false until the access point runs. */
bool ruzina_wifi_credentials(char *ssid, size_t ssid_len, char *password, size_t password_len)
{
    if (!atomic_load(&ap_ready) || ssid_len == 0 || password_len == 0) return false;
    snprintf(ssid, ssid_len, "%s", ap_ssid);
    char copy[sizeof ap_password];
    portENTER_CRITICAL(&ap_lock);
    memcpy(copy, ap_password, sizeof copy);
    portEXIT_CRITICAL(&ap_lock);
    snprintf(password, password_len, "%s", copy);
    memset(copy, 0, sizeof copy);
    return true;
}

/* WPA2: 8 to 63 printable ASCII characters, or 64 hex digits. */
static bool password_valid(const char *password)
{
    const size_t len = strlen(password);
    if (len == 64) {
        for (size_t i = 0; i < len; ++i)
            if (!isxdigit((unsigned char)password[i])) return false;
        return true;
    }
    if (len < 8 || len > 63) return false;
    for (size_t i = 0; i < len; ++i)
        if (password[i] < 0x20 || password[i] > 0x7e) return false;
    return true;
}

/* The access point's configuration with the password in force. */
static void ap_config(wifi_config_t *config)
{
    memset(config, 0, sizeof *config);
    memcpy(config->ap.ssid, ap_ssid, strlen(ap_ssid));
    config->ap.ssid_len = (uint8_t)strlen(ap_ssid);
    portENTER_CRITICAL(&ap_lock);
    const size_t password_len = strlen(ap_password);
    memcpy(config->ap.password, ap_password, password_len);
    portEXIT_CRITICAL(&ap_lock);
    config->ap.channel = AP_CHANNEL;
    config->ap.max_connection = AP_MAX_CLIENTS;
    /* WPA2 needs 8 to 63 characters (or 64 hex digits); anything else leaves the network
     * open. */
    config->ap.authmode = password_len >= 8 && password_len <= 64 ? WIFI_AUTH_WPA2_PSK : WIFI_AUTH_OPEN;
    config->ap.pmf_cfg.capable = true;
}

/* At boot, with the deck's settings and before the screen and USB audio start: the
 * password the owner set, if NVS holds a valid one. NVS reads flash, which stalls the USB
 * audio fill; read later, from the Wi-Fi task, it made one fill of 7.5 ms on s206. */
void ruzina_wifi_load_password(void)
{
    esp_err_t err = nvs_flash_init();
    if (err == ESP_ERR_NVS_NO_FREE_PAGES || err == ESP_ERR_NVS_NEW_VERSION_FOUND) {
        /* A partition never written by NVS, or by another version: start it afresh. */
        err = nvs_flash_erase();
        if (err == ESP_OK) err = nvs_flash_init();
    }
    nvs_handle_t nvs;
    char stored[sizeof ap_password] = {0};
    size_t len = sizeof stored;
    bool owner = false;
    if (err == ESP_OK && nvs_open(NVS_NAMESPACE, NVS_READONLY, &nvs) == ESP_OK) {
        owner = nvs_get_str(nvs, NVS_PASSWORD_KEY, stored, &len) == ESP_OK && password_valid(stored);
        nvs_close(nvs);
    }
    if (owner) {
        portENTER_CRITICAL(&ap_lock);
        memcpy(ap_password, stored, sizeof ap_password);
        portEXIT_CRITICAL(&ap_lock);
    }
    memset(stored, 0, sizeof stored);
    printf("wifi_ap nvs=%s password=%s\n", esp_err_to_name(err), owner ? "OWNER" : "DEFAULT");
}

/* A new password for the deck's network, or NULL for the default: kept in NVS, then the
 * running access point takes it (its stations drop and join again). On the app main task:
 * NVS writes flash, which wants an internal-RAM stack, and the caller has checked that no
 * deck plays. 0 when done, -1 for a password WPA2 cannot take, -2 when NVS failed, -3 when
 * the access point did not take it. Never logs the password. */
int32_t ruzina_wifi_change_password(const char *password)
{
    if (password && !password_valid(password)) return -1;
    if (!atomic_load(&ap_ready)) return -3;
    nvs_handle_t nvs;
    esp_err_t err = nvs_open(NVS_NAMESPACE, NVS_READWRITE, &nvs);
    if (err == ESP_OK) {
        err = password ? nvs_set_str(nvs, NVS_PASSWORD_KEY, password) : nvs_erase_key(nvs, NVS_PASSWORD_KEY);
        if (err == ESP_ERR_NVS_NOT_FOUND) err = ESP_OK;
        if (err == ESP_OK) err = nvs_commit(nvs);
        nvs_close(nvs);
    }
    if (err != ESP_OK) {
        printf("wifi_ap password_change nvs=%s\n", esp_err_to_name(err));
        return -2;
    }
    portENTER_CRITICAL(&ap_lock);
    memset(ap_password, 0, sizeof ap_password);
    memcpy(ap_password, password ? password : default_password, strlen(password ? password : default_password));
    portEXIT_CRITICAL(&ap_lock);
    wifi_config_t config;
    ap_config(&config);
    err = esp_wifi_set_config(WIFI_IF_AP, &config);
    memset(&config, 0, sizeof config);
    printf("wifi_ap password_change=%s source=%s\n", esp_err_to_name(err), password ? "OWNER" : "DEFAULT");
    return err == ESP_OK ? 0 : -3;
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
    wifi_config_t config;
    ap_config(&config);
    err = esp_wifi_set_mode(WIFI_MODE_AP);
    if (err == ESP_OK) err = esp_wifi_set_config(WIFI_IF_AP, &config);
    if (err == ESP_OK) err = esp_wifi_start();
    printf("wifi_ap start=%s ssid=%s channel=%d auth=%s\n", esp_err_to_name(err), ap_ssid, AP_CHANNEL,
           config.ap.authmode == WIFI_AUTH_WPA2_PSK ? "wpa2" : "open");
    memset(&config, 0, sizeof config);
    if (err == ESP_OK) {
        /* The page shows which deck it drives: the network's name. */
        ruzina_web_set_name((const uint8_t *)ap_ssid, strlen(ap_ssid));
        atomic_store(&ap_ready, true);
    }
    return err;
}

/* The connections' sockets for web_client.h: never waiting. */
static int web_recv(int sock, uint8_t *buffer, size_t len) { return recv(sock, buffer, len, MSG_DONTWAIT); }
static int web_send(int sock, const uint8_t *buffer, size_t len) { return send(sock, buffer, len, MSG_DONTWAIT); }
static void web_close(int sock)
{
    shutdown(sock, SHUT_RDWR);
    close(sock);
}
/* Differs on each start, so the page notices a reboot. */
static uint32_t boot;
/* A firmware bundle streams to ruzina-web (deck_ota.rs): the signed header and up to a
 * slot's 6 MiB. */
#define WEB_STREAM_MAX (188 + 0x600000)
static int web_stream_open(const uint8_t *head, size_t len) { return ruzina_web_stream_open(boot, head, len); }
static int web_stream_feed(const uint8_t *bytes, size_t len) { return ruzina_web_stream_feed(bytes, len); }
static void web_stream_cancel(void) { ruzina_web_stream_cancel(); }
#include "web_client.h"

/* Connections the server serves at once. */
#define CLIENTS 4

static atomic_uint dropped;
static size_t response_cap;

/* A complete request's answer, from ruzina-web, into the connection's own buffer. A
 * streamed body is finished there (or its refusal answered). */
static size_t answer(struct client *c)
{
    if (c->streaming) {
        c->streaming = false;
        return ruzina_web_stream_finish(c->stream_status, c->out, response_cap);
    }
    return ruzina_web_handle(boot, c->buffer, c->len, c->out, response_cap);
}

/* One task serves up to CLIENTS connections at once. It answers each as soon as its
 * request is in, and sends each answer in pieces as its socket takes them, so a slow
 * client holds no other back. ruzina-web still sees one request at a time. A connection
 * that has not sent its request within CLIENT_TIMEOUT_US, or taken its answer within
 * SEND_TIMEOUT_US, is dropped. */
static void server_task(void *unused)
{
    (void)unused;
    response_cap = ruzina_web_largest_asset() + 1024;
    /* Each connection keeps its own request and answer: an answer stays until it is sent. */
    struct client conns[CLIENTS];
    bool ok = true;
    for (int i = 0; i < CLIENTS; ++i) {
        conns[i] = (struct client){
            .sock = -1,
            .buffer = heap_caps_malloc(REQUEST_MAX, MALLOC_CAP_SPIRAM),
            .out = heap_caps_malloc(response_cap, MALLOC_CAP_SPIRAM),
        };
        ok = ok && conns[i].buffer != NULL && conns[i].out != NULL;
    }
    int listener = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    struct sockaddr_in address = {.sin_family = AF_INET, .sin_port = htons(80), .sin_addr.s_addr = htonl(INADDR_ANY)};
    const int yes = 1;
    setsockopt(listener, SOL_SOCKET, SO_REUSEADDR, &yes, sizeof yes);
    ok = ok && listener >= 0 && bind(listener, (struct sockaddr *)&address, sizeof address) == 0 &&
         listen(listener, 4) == 0;
    printf("web_remote listen=%s response_cap=%u clients=%d send_slice=%d\n", ok ? "ok" : "failed",
           (unsigned)response_cap, CLIENTS, WEB_SEND_SLICE);
    if (!ok) {
        /* Give back whatever did start: the buffers and the socket. */
        if (listener >= 0) close(listener);
        for (int i = 0; i < CLIENTS; ++i) {
            heap_caps_free(conns[i].buffer);
            heap_caps_free(conns[i].out);
        }
        atomic_store(&server_state, SERVER_FAILED);
        vTaskDelete(NULL);
        return;
    }
    boot = esp_random();
    atomic_store(&server_state, SERVER_LISTENING);
    for (;;) {
        /* A connection still sending its request waits to be read; one with an answer
         * waits until its socket takes more. */
        fd_set reads, writes;
        FD_ZERO(&reads);
        FD_ZERO(&writes);
        int top = -1;
        bool room = false;
        for (int i = 0; i < CLIENTS; ++i) {
            if (conns[i].sock >= 0) {
                FD_SET(conns[i].sock, conns[i].out_len ? &writes : &reads);
                if (conns[i].sock > top) top = conns[i].sock;
            } else {
                room = true;
            }
        }
        /* With every slot busy, new connections wait in the backlog. */
        if (room) {
            FD_SET(listener, &reads);
            if (listener > top) top = listener;
        }
        struct timeval tick = {.tv_sec = 0, .tv_usec = 200000};
        int ready = select(top + 1, &reads, &writes, NULL, &tick);
        const int64_t now = esp_timer_get_time();
        if (ready > 0 && room && FD_ISSET(listener, &reads)) {
            struct sockaddr_storage peer;
            socklen_t peer_len = sizeof peer;
            int sock = accept(listener, (struct sockaddr *)&peer, &peer_len);
            if (sock >= 0) {
                for (int i = 0; i < CLIENTS; ++i) {
                    if (conns[i].sock < 0) {
                        conns[i].sock = sock;
                        conns[i].len = 0;
                        conns[i].want = 0;
                        conns[i].start = now;
                        break;
                    }
                }
            }
        }
        for (int i = 0; i < CLIENTS; ++i) {
            struct client *c = &conns[i];
            if (c->sock < 0) continue;
            const bool readable = ready > 0 && FD_ISSET(c->sock, &reads);
            const bool writable = ready > 0 && FD_ISSET(c->sock, &writes);
            switch (client_step(c, readable, writable, now, answer)) {
                case CLIENT_ANSWERED: {
                    const unsigned took = (unsigned)(esp_timer_get_time() - c->start);
                    atomic_fetch_add(&requests, 1);
                    if (took > atomic_load(&request_max_us)) atomic_store(&request_max_us, took);
                    /* A new image is in its slot and set to boot: the answer has gone. */
                    if (ruzina_web_restart_due()) {
                        printf("ota restart\n");
                        vTaskDelay(pdMS_TO_TICKS(1000));
                        esp_restart();
                    }
                    break;
                }
                case CLIENT_DROPPED: atomic_fetch_add(&dropped, 1); break;
                case CLIENT_WAITING: break;
            }
        }
    }
}

static void wifi_task(void *unused)
{
    (void)unused;
    ruzina_ota_report();
    lower_hosted_tasks("start");
    ESP_ERROR_CHECK_WITHOUT_ABORT(esp_netif_init());
    ESP_ERROR_CHECK_WITHOUT_ABORT(esp_event_loop_create_default());
    esp_err_t err = start_access_point();
    if (err == ESP_OK &&
        xTaskCreatePinnedToCore(server_task, "web_remote", 6144, NULL, LOW_PRIORITY, NULL, 1) != pdPASS) {
        err = ESP_ERR_NO_MEM;
    }
    /* By now the link is up: the C6's firmware version. */
    esp_hosted_coprocessor_fwver_t version = {0};
    if (esp_hosted_get_coprocessor_fwversion(&version) == ESP_OK) {
        printf("wifi c6_firmware=%" PRIu32 ".%" PRIu32 ".%" PRIu32 "\n",
               version.major1, version.minor1, version.patch1);
    }
    if (err != ESP_OK) {
        printf("wifi error=%s\n", esp_err_to_name(err));
        /* No Web Remote: a new image that cannot take the next one goes back. */
        ruzina_ota_settle(false);
        vTaskDelete(NULL);
        return;
    }
    for (unsigned reports = 1;; ++reports) {
        vTaskDelay(pdMS_TO_TICKS(REPORT_PERIOD_MS));
        lower_hosted_tasks("report");
        /* A new image is kept once it has run 30 s with the server listening; it goes
         * back if the server failed, or has not started within three reports. */
        const int state = atomic_load(&server_state);
        if (state != SERVER_STARTING || reports >= 3) ruzina_ota_settle(state == SERVER_LISTENING);
        static const char *const server_names[] = {"starting", "listening", "failed"};
        printf("wifi_ap server=%s clients=%d requests=%u request_max_us=%u dropped=%u internal_free=%u\n",
               server_names[atomic_load(&server_state)], atomic_load(&clients), atomic_load(&requests),
               atomic_exchange(&request_max_us, 0),
               atomic_load(&dropped),
               (unsigned)heap_caps_get_free_size(MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT));
    }
}

esp_err_t ruzina_wifi_start(void)
{
    /* The screen's core, below the decoders and the screen. */
    return xTaskCreatePinnedToCore(wifi_task, "wifi", 6144, NULL, LOW_PRIORITY, NULL, 1) == pdPASS
               ? ESP_OK
               : ESP_ERR_NO_MEM;
}
