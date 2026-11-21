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
#include "freertos/semphr.h"
#if CONFIG_RUZINA_UAC_PROFILE && CONFIG_RUZINA_SLINT_UI
#include <stdatomic.h>
#include <string.h>
#include "driver/ppa.h"
#include "esp_cache.h"
#include "esp_heap_caps.h"
#endif

#if ESP_IDF_VERSION != ESP_IDF_VERSION_VAL(6, 1, 0)
#error "This compile experiment is pinned to ESP-IDF 6.1"
#endif

/* Scalar ABI only: no Rust layout, allocations, pointers or exceptions cross it. */
extern int16_t ruzina_legacy_pitch_centipercent(uint16_t raw);

#if CONFIG_RUZINA_UAC_PROFILE
/* Profile builds: times the render graph before USB audio and the media load start
 * (render_bench.rs). Each case runs on a task of its own on core 0 at the USB client's
 * priority, with the graph on its stack in internal RAM; between cases core 0's idle
 * task runs, for the task watchdog. */
extern uint32_t ruzina_legacy_render_bench(uint32_t render_case, uint32_t *out);
typedef struct { uint32_t render_case, ok, words[3]; SemaphoreHandle_t done; } render_bench_t;
static void render_bench_task(void *arg)
{
    render_bench_t *bench = arg;
    bench->ok = ruzina_legacy_render_bench(bench->render_case, bench->words);
    xSemaphoreGive(bench->done);
    vTaskDelete(NULL);
}
static void render_bench(void)
{
    static const char *const names[] = {"graph", "graph_echo", "internal_ring", "psram_ring", "psram_ring_acquire",
        "resampler", "eq", "filter_center", "limiter_pcm", "graph_one_deck", "graph_no_fx", "keylock",
        "graph_keylock", "keylock_internal"};
    render_bench_t bench = {.done = xSemaphoreCreateBinary()};
    if (!bench.done) { printf("render_bench result=NO_SEMAPHORE\n"); return; }
    for (uint32_t i = 0; i < sizeof(names) / sizeof(names[0]); ++i) {
        bench.render_case = i; bench.ok = 0;
        if (xTaskCreatePinnedToCore(render_bench_task, "render_bench", 16384, &bench, 4, NULL, 0) != pdPASS) {
            printf("render_bench case=%s result=TASK_FAILED\n", names[i]); break;
        }
        xSemaphoreTake(bench.done, portMAX_DELAY);
        if (bench.ok) {
            printf("render_bench case=%s fills=300 frames=288 mean_us=%" PRIu32 " min_us=%" PRIu32 " max_us=%" PRIu32 "\n",
                   names[i], bench.words[0], bench.words[1], bench.words[2]);
        } else {
            printf("render_bench case=%s result=FAILED\n", names[i]);
        }
        fflush(stdout);
        vTaskDelay(pdMS_TO_TICKS(50));
    }
    vSemaphoreDelete(bench.done);
}

#if CONFIG_RUZINA_SLINT_UI
/* The same graph while core 1 keeps doing one kind of the screen's memory work, over and
 * over, on a lane of two frame buffers of its own: which kind slows the fill (s163). The
 * screen does each in bursts; here it never stops, so each is at its worst. */
enum { LOAD_NONE, LOAD_MOVE, LOAD_DRAW, LOAD_DRAW_FLUSH, LOAD_FLUSH, LOAD_COPY, LOADS };
enum { LANE_X = 82, LANE_Y = 46, LANE_W = 872, LANE_H = 201, FRAME_W = 1024, FRAME_H = 600 };
typedef struct {
    uint32_t kind, rounds;
    _Atomic bool stop;
    uint16_t *a, *b;
    ppa_client_handle_t ppa;
    SemaphoreHandle_t done;
} screen_load_t;

static void screen_load_task(void *arg)
{
    screen_load_t *l = arg;
    uint16_t ink = 1;
    while (!atomic_load_explicit(&l->stop, memory_order_acquire)) {
        uint16_t *rows = l->b + LANE_Y * FRAME_W;
        switch (l->kind) {
        case LOAD_MOVE: {
            /* A lane scrolled by 8 columns, as ruzina_panel_move does it. */
            ppa_srm_oper_config_t move = {
                .in = {.buffer = l->a, .pic_w = FRAME_W, .pic_h = FRAME_H, .block_w = LANE_W - 8, .block_h = LANE_H,
                       .block_offset_x = LANE_X + 8, .block_offset_y = LANE_Y, .srm_cm = PPA_SRM_COLOR_MODE_RGB565},
                .out = {.buffer = l->b, .buffer_size = FRAME_W * FRAME_H * 2, .pic_w = FRAME_W, .pic_h = FRAME_H,
                        .block_offset_x = LANE_X, .block_offset_y = LANE_Y, .srm_cm = PPA_SRM_COLOR_MODE_RGB565},
                .rotation_angle = PPA_SRM_ROTATION_ANGLE_0, .scale_x = 1.0f, .scale_y = 1.0f,
                .mode = PPA_TRANS_MODE_BLOCKING,
            };
            if (ppa_do_scale_rotate_mirror(l->ppa, &move) != ESP_OK) atomic_store(&l->stop, true);
            break;
        }
        case LOAD_DRAW:
        case LOAD_DRAW_FLUSH:
            /* A whole lane's columns written by the CPU, row by row. */
            for (unsigned y = 0; y < LANE_H; ++y)
                for (unsigned x = 0; x < LANE_W; ++x) rows[y * FRAME_W + LANE_X + x] = ink;
            ++ink;
            if (l->kind == LOAD_DRAW) break;
            /* And written back for the panel. */
            __attribute__((fallthrough));
        case LOAD_FLUSH:
            esp_cache_msync(rows, FRAME_W * LANE_H * 2, ESP_CACHE_MSYNC_FLAG_DIR_C2M);
            break;
        case LOAD_COPY:
            memcpy(rows, l->a + LANE_Y * FRAME_W, FRAME_W * LANE_H * 2);
            break;
        default:
            break;
        }
        ++l->rounds;
    }
    xSemaphoreGive(l->done);
    vTaskDelete(NULL);
}

static void render_contention(void)
{
    static const char *const loads[LOADS] = {"none", "ppa_move", "cpu_draw", "cpu_draw_flush", "flush_only", "cpu_copy"};
    /* The PSRAM-ring graph a plain fill runs, and the graph with both keylocks. */
    static const uint32_t cases[] = {4, 12};
    static const char *const case_names[] = {"psram_ring_acquire", "graph_keylock"};
    screen_load_t load = {.done = xSemaphoreCreateBinary()};
    render_bench_t bench = {.done = xSemaphoreCreateBinary()};
    load.a = heap_caps_aligned_calloc(64, FRAME_W * FRAME_H, 2, MALLOC_CAP_SPIRAM);
    load.b = heap_caps_aligned_calloc(64, FRAME_W * FRAME_H, 2, MALLOC_CAP_SPIRAM);
    ppa_client_config_t client = {.oper_type = PPA_OPERATION_SRM, .max_pending_trans_num = 1};
    if (!load.done || !bench.done || !load.a || !load.b || ppa_register_client(&client, &load.ppa) != ESP_OK) {
        printf("render_contention result=NO_RESOURCES\n");
        goto done;
    }
    esp_cache_msync(load.a, FRAME_W * FRAME_H * 2, ESP_CACHE_MSYNC_FLAG_DIR_C2M);
    for (uint32_t kind = 0; kind < LOADS; ++kind) {
        for (unsigned c = 0; c < sizeof(cases) / sizeof(cases[0]); ++c) {
            load.kind = kind; load.rounds = 0;
            atomic_store_explicit(&load.stop, false, memory_order_release);
            if (kind != LOAD_NONE &&
                xTaskCreatePinnedToCore(screen_load_task, "screen_load", 4096, &load, 3, NULL, 1) != pdPASS) {
                printf("render_contention load=%s result=TASK_FAILED\n", loads[kind]);
                goto done;
            }
            vTaskDelay(pdMS_TO_TICKS(20));
            const int64_t started = esp_timer_get_time();
            bench.render_case = cases[c]; bench.ok = 0;
            if (xTaskCreatePinnedToCore(render_bench_task, "render_bench", 16384, &bench, 4, NULL, 0) != pdPASS) {
                printf("render_contention load=%s result=TASK_FAILED\n", loads[kind]);
                atomic_store_explicit(&load.stop, true, memory_order_release);
                if (kind != LOAD_NONE) xSemaphoreTake(load.done, portMAX_DELAY);
                goto done;
            }
            xSemaphoreTake(bench.done, portMAX_DELAY);
            const int64_t took = esp_timer_get_time() - started;
            atomic_store_explicit(&load.stop, true, memory_order_release);
            if (kind != LOAD_NONE) xSemaphoreTake(load.done, portMAX_DELAY);
            /* Rounds of the load a second, against about 60 moves and draws the screen does. */
            const uint32_t per_s = took > 0 ? (uint32_t)((uint64_t)load.rounds * 1000000u / (uint64_t)took) : 0;
            if (bench.ok) {
                printf("render_contention load=%s case=%s fills=300 mean_us=%" PRIu32 " min_us=%" PRIu32
                       " max_us=%" PRIu32 " load_per_s=%" PRIu32 "\n",
                       loads[kind], case_names[c], bench.words[0], bench.words[1], bench.words[2], per_s);
            } else {
                printf("render_contention load=%s case=%s result=FAILED\n", loads[kind], case_names[c]);
            }
            fflush(stdout);
            vTaskDelay(pdMS_TO_TICKS(50));
        }
    }
done:
    if (load.ppa) ppa_unregister_client(load.ppa);
    heap_caps_free(load.a);
    heap_caps_free(load.b);
    if (load.done) vSemaphoreDelete(load.done);
    if (bench.done) vSemaphoreDelete(bench.done);
}
#endif
#endif

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
#if CONFIG_RUZINA_DECK_ENGINE
    /* Erasing flash stalls everything that runs from flash and PSRAM, the USB audio fill
     * included. Before the screen and USB audio start, the Hot Cue journal erases its
     * stale spare sectors, so that saves later need no erase. */
    extern uint32_t ruzina_legacy_hotcues_erase_spares(void);
    const int64_t spares_start = esp_timer_get_time();
    const uint32_t spares = ruzina_legacy_hotcues_erase_spares();
    printf("hotcue_spares erased=%" PRIu32 " us=%" PRId64 "\n", spares, esp_timer_get_time() - spares_start);
    /* The deck's settings, read the same way; the screen takes their brightness. */
    extern uint32_t ruzina_legacy_settings_load(void);
    const int64_t settings_start = esp_timer_get_time();
    const uint32_t settings = ruzina_legacy_settings_load();
    printf("settings state=%" PRIu32 " backlight_percent=%" PRIu32 " spare_erased=%" PRIu32 " us=%" PRId64 "\n",
           (settings >> 8) & 0xf, settings & 0xff, (settings >> 12) & 1, esp_timer_get_time() - settings_start);
#if CONFIG_RUZINA_DISPLAY_TOUCH || CONFIG_RUZINA_LOAD_DISPLAY
    extern void ruzina_display_backlight(uint32_t percent);
    ruzina_display_backlight(settings & 0xff);
#endif
#if CONFIG_RUZINA_UAC_PROFILE
    render_bench();
#if CONFIG_RUZINA_SLINT_UI
    render_contention();
#endif
#endif
    fflush(stdout);
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
#if CONFIG_RUZINA_USB_HOST_ENUM
    /* After the screen, so controller actions find it ready. */
    extern esp_err_t ruzina_usb_host_enum_start(void);
    esp_err_t usb_err = ruzina_usb_host_enum_start();
    if (usb_err != ESP_OK) {
        printf("usb_host=INIT_FAILED error=%s\n", esp_err_to_name(usb_err));
    }
#endif
#if CONFIG_RUZINA_WIFI_REMOTE
    /* Last, once USB audio runs: the deck's Wi-Fi and the Web Remote. */
    extern esp_err_t ruzina_wifi_start(void);
    if (ruzina_wifi_start() != ESP_OK) {
        printf("wifi=INIT_FAILED\n");
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
#if CONFIG_RUZINA_DECK_ENGINE
        /* The deck's settings are saved here, on this internal-RAM stack, card or not. */
        extern void ruzina_legacy_settings_service(void);
        ruzina_legacy_settings_service();
#endif
    }
}
