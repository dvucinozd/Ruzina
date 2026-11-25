#include <inttypes.h>
#include <stdio.h>
#include <string.h>
#include "driver/gpio.h"
#include "driver/i2c_master.h"
#include "driver/ledc.h"
#include "esp_cache.h"
#include "esp_heap_caps.h"
#include "esp_lcd_mipi_dsi.h"
#include "esp_lcd_panel_io.h"
#include "esp_lcd_panel_ops.h"
#if CONFIG_RUZINA_SLINT_UI
#include "driver/ppa.h"
#include "esp_private/dma2d.h"
#include "hal/dma2d_types.h"
#include "soc/dma2d_channel.h"
#endif
#include "esp_ldo_regulator.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "freertos/semphr.h"
#include "sdkconfig.h"
#include "new_panel_commands.h"

/* Single display owner. GPIO21/22 remain untouched; no audio/C6 owner. The
 * diagnostic profile adds touch; the load profile shares only the panel. */
static esp_ldo_channel_handle_t phy_power;
static esp_lcd_dsi_bus_handle_t bus;
static esp_lcd_panel_io_handle_t io;
static esp_lcd_panel_handle_t panel;
static uint16_t *frame;
static bool pwm_ready;
#if CONFIG_RUZINA_SLINT_UI
/* The deck screen draws into the buffer the panel does not show, then swaps them at the
   end of a refresh, so the panel never shows a half-drawn frame. */
static uint16_t *back_frame;
static SemaphoreHandle_t refreshed;
static volatile uint32_t refreshes;
static ppa_client_handle_t mover;
/* The lanes' task, woken at each refresh (`lane_task`). */
static TaskHandle_t lane_task_handle;

static IRAM_ATTR bool frame_done(esp_lcd_panel_handle_t panel_handle, esp_lcd_dpi_panel_event_data_t *event, void *context)
{
    (void)panel_handle; (void)event; (void)context;
    BaseType_t woken = pdFALSE;
    refreshes++;
    xSemaphoreGiveFromISR(refreshed, &woken);
    if (lane_task_handle) vTaskNotifyGiveFromISR(lane_task_handle, &woken);
    return woken == pdTRUE;
}

uint16_t *ruzina_panel_back(void) { return back_frame; }

/* Panel refreshes since start, about 61 a second: the clock the waveforms scroll by. */
uint32_t ruzina_panel_refreshes(void) { return refreshes; }

/* Copies the block at (x + dx, y), `w - dx` by `h` pixels, of frame buffer `from` to
   (x, y) of frame buffer `to`, in hardware: the waveform scrolls by `dx` columns without
   the CPU touching its pixels. The PPA driver writes `from` back from cache first and
   invalidates the rows of `to`, so the caller writes those rows back before. */
static bool mover_ready(void)
{
    if (!mover) {
        ppa_client_config_t client = {.oper_type = PPA_OPERATION_SRM, .max_pending_trans_num = 1};
        if (ppa_register_client(&client, &mover) != ESP_OK) return false;
    }
    return true;
}

/* A lane move as a plain 2D-DMA memory-to-memory copy. The PPA's SRM engine moved a lane
   (811 by 220 pixels) in 8.1 ms on s190p, its cache work taking under 0.05 ms: the scaling
   engine is the bottleneck, not the memory. The 2D-DMA copies the same block without it,
   in 2.5 ms on s191p. Up to six copies at a time, one in each slot, from one task at a
   time: the lanes' task (`lane_task`), or before it the deck screen. Each slot's
   descriptors and transaction are kept. Falls back to the PPA if the 2D-DMA cannot be
   had. */
#define COPY_SLOTS 6
static dma2d_pool_handle_t copy_pool;
static dma2d_descriptor_t *copy_tx[COPY_SLOTS], *copy_rx[COPY_SLOTS];
static void *copy_slot[COPY_SLOTS];
static dma2d_trans_config_t copy_job[COPY_SLOTS];
static SemaphoreHandle_t copy_done[COPY_SLOTS];
static bool copy_busy[COPY_SLOTS];
static bool copy_failed;
#if CONFIG_RUZINA_UAC_PROFILE
static volatile int64_t copy_begun[COPY_SLOTS], copy_ended[COPY_SLOTS];
#endif

static bool copy_eof(dma2d_channel_handle_t chan, dma2d_event_data_t *event, void *user)
{
    (void)chan;
    (void)event;
    const uintptr_t slot = (uintptr_t)user;
#if CONFIG_RUZINA_UAC_PROFILE
    copy_ended[slot] = esp_timer_get_time();
#endif
    BaseType_t woken = pdFALSE;
    xSemaphoreGiveFromISR(copy_done[slot], &woken);
    return woken == pdTRUE;
}

static bool copy_picked(uint32_t num, const dma2d_trans_channel_info_t *chans, void *user)
{
    if (num != 2) return false;
    const uintptr_t slot = (uintptr_t)user;
    dma2d_channel_handle_t tx = chans[0].dir == DMA2D_CHANNEL_DIRECTION_TX ? chans[0].chan : chans[1].chan;
    dma2d_channel_handle_t rx = chans[0].dir == DMA2D_CHANNEL_DIRECTION_TX ? chans[1].chan : chans[0].chan;
    dma2d_trigger_t trig = {.periph = DMA2D_TRIG_PERIPH_M2M, .periph_sel_id = SOC_DMA2D_TRIG_PERIPH_M2M_TX};
    dma2d_connect(tx, &trig);
    trig.periph_sel_id = SOC_DMA2D_TRIG_PERIPH_M2M_RX;
    dma2d_connect(rx, &trig);
    dma2d_transfer_ability_t ability = {
        .access_ext_mem = true, .data_burst_length = 128,
        .desc_burst_en = true, .mb_size = DMA2D_MACRO_BLOCK_SIZE_NONE,
    };
    dma2d_set_transfer_ability(tx, &ability);
    dma2d_set_transfer_ability(rx, &ability);
    dma2d_rx_event_callbacks_t cbs = {.on_recv_eof = copy_eof};
    dma2d_register_rx_event_callbacks(rx, &cbs, user);
    dma2d_set_desc_addr(tx, (intptr_t)copy_tx[slot]);
    dma2d_set_desc_addr(rx, (intptr_t)copy_rx[slot]);
    dma2d_start(tx);
    dma2d_start(rx);
    return false;
}

static bool copy_ready(void)
{
    if (copy_failed) return false;
    if (!copy_pool) {
        bool made = true;
        for (int slot = 0; slot < COPY_SLOTS; slot++) {
            copy_tx[slot] = heap_caps_aligned_calloc(64, 1, 64, MALLOC_CAP_INTERNAL | MALLOC_CAP_DMA);
            copy_rx[slot] = heap_caps_aligned_calloc(64, 1, 64, MALLOC_CAP_INTERNAL | MALLOC_CAP_DMA);
            copy_slot[slot] = heap_caps_calloc(1, SIZEOF_DMA2D_TRANS_T, MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT);
            copy_done[slot] = xSemaphoreCreateBinary();
            made = made && copy_tx[slot] && copy_rx[slot] && copy_slot[slot] && copy_done[slot];
        }
        dma2d_pool_config_t pool = {.pool_id = 0};
        if (!made || dma2d_acquire_pool(&pool, &copy_pool) != ESP_OK) {
            copy_failed = true;
            printf("panel_copy dma2d=UNAVAILABLE\n");
            return false;
        }
    }
    return true;
}

/* Waits for the copy in `slot` to end. A copy that never ends stops the 2D-DMA for good. */
static bool copy_end(uint32_t slot)
{
    if (!copy_busy[slot]) return false;
    copy_busy[slot] = false;
    if (xSemaphoreTake(copy_done[slot], pdMS_TO_TICKS(50)) != pdTRUE) {
        copy_failed = true;
        printf("panel_copy dma2d=TIMEOUT\n");
        return false;
    }
    return true;
}

/* Starts copying the w by h block at (fx, fy) of `from` to (tx, ty) of `to`, in `slot`;
   both are 1024 pixels to a row, `from_h` and `to_h` rows. A frame buffer's rows are synced
   first: the CPU must not write the rows written until `copy_end`, as their cache lines are
   dropped now and one written back later would put old pixels over the copy's. A lane
   cache's are not (`lane_task`). */
static bool copy_begin(uint32_t slot, const uint16_t *from, uint32_t from_h, uint32_t fx, uint32_t fy,
                       uint16_t *to, uint32_t to_h, uint32_t tx, uint32_t ty, uint32_t w, uint32_t h)
{
    if (slot >= COPY_SLOTS || !copy_ready()) return false;
    if (copy_busy[slot] && !copy_end(slot)) return false;
    const uint32_t pbyte = DMA2D_DESCRIPTOR_PBYTE_2B0_PER_PIXEL;
    *copy_tx[slot] = (dma2d_descriptor_t){
        .vb_size = h, .hb_length = w, .err_eof = 0, .dma2d_en = 1, .suc_eof = 1,
        .owner = DMA2D_DESCRIPTOR_BUFFER_OWNER_DMA, .va_size = from_h, .ha_length = 1024, .pbyte = pbyte,
        .y = fy, .x = fx, .mode = DMA2D_DESCRIPTOR_BLOCK_RW_MODE_SINGLE, .buffer = (void *)from, .next = NULL,
    };
    *copy_rx[slot] = (dma2d_descriptor_t){
        .vb_size = h, .hb_length = w, .err_eof = 0, .dma2d_en = 1, .suc_eof = 0,
        .owner = DMA2D_DESCRIPTOR_BUFFER_OWNER_DMA, .va_size = to_h, .ha_length = 1024, .pbyte = pbyte,
        .y = ty, .x = tx, .mode = DMA2D_DESCRIPTOR_BLOCK_RW_MODE_SINGLE, .buffer = to, .next = NULL,
    };
    esp_cache_msync(copy_tx[slot], 64, ESP_CACHE_MSYNC_FLAG_DIR_C2M);
    esp_cache_msync(copy_rx[slot], 64, ESP_CACHE_MSYNC_FLAG_DIR_C2M);
    /* The rows read go to memory first; the rows written are dropped from the cache. */
    if (from_h == 600) {
        esp_cache_msync((void *)(from + fy * 1024), 1024 * h * 2, ESP_CACHE_MSYNC_FLAG_DIR_C2M | ESP_CACHE_MSYNC_FLAG_UNALIGNED);
        esp_cache_msync((void *)(to + ty * 1024), 1024 * h * 2, ESP_CACHE_MSYNC_FLAG_DIR_M2C);
    }
    copy_job[slot] = (dma2d_trans_config_t){
        .tx_channel_num = 1, .rx_channel_num = 1, .channel_flags = DMA2D_CHANNEL_FUNCTION_FLAG_SIBLING,
        .on_job_picked = copy_picked, .user_config = (void *)(uintptr_t)slot,
    };
    xSemaphoreTake(copy_done[slot], 0);
#if CONFIG_RUZINA_UAC_PROFILE
    copy_begun[slot] = esp_timer_get_time();
#endif
    if (dma2d_enqueue(copy_pool, &copy_job[slot], (dma2d_trans_t *)copy_slot[slot]) != ESP_OK) return false;
    copy_busy[slot] = true;
    return true;
}

/* Copies the block at once, in slot 0. */
static bool dma2d_block_copy(const uint16_t *from, uint32_t fx, uint32_t fy, uint16_t *to, uint32_t tx,
                             uint32_t ty, uint32_t w, uint32_t h)
{
    return copy_begin(0, from, 600, fx, fy, to, 600, tx, ty, w, h) && copy_end(0);
}

int32_t ruzina_panel_move(const uint16_t *from, uint16_t *to, uint32_t x, uint32_t y,
                          uint32_t w, uint32_t h, uint32_t dx)
{
    if (dx >= w) return -1;
#if CONFIG_RUZINA_UAC_PROFILE
    static uint32_t dma_moves, dma_samples, dma_us, dma_max_us;
    const int64_t dma_start = esp_timer_get_time();
#endif
    if (dma2d_block_copy(from, x + dx, y, to, x, y, w - dx, h)) {
#if CONFIG_RUZINA_UAC_PROFILE
        /* Profile images: the copy's time, the means of every 16th of 32 samples. */
        if ((dma_moves++ & 15) == 0) {
            const uint32_t took = (uint32_t)(esp_timer_get_time() - dma_start);
            dma_us += took;
            if (took > dma_max_us) dma_max_us = took;
            if (++dma_samples == 32) {
                printf("panel_move via=dma2d samples=32 w=%" PRIu32 " h=%" PRIu32 " copy_us=%" PRIu32
                       " copy_max_us=%" PRIu32 "\n", w - dx, h, dma_us / 32, dma_max_us);
                dma_samples = dma_us = dma_max_us = 0;
            }
        }
#endif
        return 0;
    }
    if (!mover_ready()) return -1;
    ppa_srm_oper_config_t move = {
        .in = {.buffer = from, .pic_w = 1024, .pic_h = 600, .block_w = w - dx, .block_h = h,
               .block_offset_x = x + dx, .block_offset_y = y, .srm_cm = PPA_SRM_COLOR_MODE_RGB565},
        .out = {.buffer = to, .buffer_size = 1024 * 600 * 2, .pic_w = 1024, .pic_h = 600,
                .block_offset_x = x, .block_offset_y = y, .srm_cm = PPA_SRM_COLOR_MODE_RGB565},
        .rotation_angle = PPA_SRM_ROTATION_ANGLE_0, .scale_x = 1.0f, .scale_y = 1.0f,
        .mode = PPA_TRANS_MODE_BLOCKING,
    };
#if CONFIG_RUZINA_UAC_PROFILE
    /* Profile images: every 16th move, where its time goes. The cache write-back of the
       rows read and the invalidation of the rows written run first, timed, as the PPA
       driver runs them; the driver's own then find the lines done, so the move's time
       that follows is the copy itself. The means print every 32 samples. */
    static uint32_t moves, samples, c2m_us, m2c_us, copy_us, copy_max_us;
    if ((moves++ & 15) == 0) {
        const int64_t t0 = esp_timer_get_time();
        esp_cache_msync((void *)(from + y * 1024), 1024 * h * 2, ESP_CACHE_MSYNC_FLAG_DIR_C2M | ESP_CACHE_MSYNC_FLAG_UNALIGNED);
        const int64_t t1 = esp_timer_get_time();
        esp_cache_msync((void *)(to + y * 1024), 1024 * h * 2, ESP_CACHE_MSYNC_FLAG_DIR_M2C);
        const int64_t t2 = esp_timer_get_time();
        const bool ok = ppa_do_scale_rotate_mirror(mover, &move) == ESP_OK;
        const uint32_t copy = (uint32_t)(esp_timer_get_time() - t2);
        c2m_us += (uint32_t)(t1 - t0);
        m2c_us += (uint32_t)(t2 - t1);
        copy_us += copy;
        if (copy > copy_max_us) copy_max_us = copy;
        if (++samples == 32) {
            printf("panel_move via=ppa samples=32 w=%" PRIu32 " h=%" PRIu32 " c2m_us=%" PRIu32 " m2c_us=%" PRIu32
                   " copy_us=%" PRIu32 " copy_max_us=%" PRIu32 "\n",
                   w - dx, h, c2m_us / 32, m2c_us / 32, copy_us / 32, copy_max_us);
            samples = c2m_us = m2c_us = copy_us = copy_max_us = 0;
        }
        return ok ? 0 : -1;
    }
#endif
    return ppa_do_scale_rotate_mirror(mover, &move) == ESP_OK ? 0 : -1;
}

/* Starts moving the w by h block at (x, y) of frame buffer `to` left by dx columns, from the
   same block of frame buffer `from`, by the 2D-DMA in `slot` (0 or 1); 0 once started, -1
   if not (the caller then moves it with ruzina_panel_move). Until ruzina_panel_move_finish
   the CPU must leave the block's rows of `to` alone. */
int32_t ruzina_panel_move_start(const uint16_t *from, uint16_t *to, uint32_t x, uint32_t y,
                                uint32_t w, uint32_t h, uint32_t dx, uint32_t slot)
{
    if (dx >= w) return -1;
    return copy_begin(slot, from, 600, x + dx, y, to, 600, x, y, w - dx, h) ? 0 : -1;
}

/* Waits for the move started in `slot`: 0 once it is done, -1 if it failed (the caller then
   draws the block whole). */
int32_t ruzina_panel_move_finish(uint32_t slot)
{
    if (slot >= COPY_SLOTS) return -1;
#if CONFIG_RUZINA_UAC_PROFILE
    /* Profile images: each copy's time from its start to its end, and how long the screen
       still waited for it; the means of every 8th of 32 samples. */
    static uint32_t finishes, samples, copy_us, wait_us, wait_max_us;
    const int64_t waiting = esp_timer_get_time();
    const bool done = copy_end(slot);
    if (done && (finishes++ & 7) == 0) {
        const uint32_t waited = (uint32_t)(esp_timer_get_time() - waiting);
        copy_us += (uint32_t)(copy_ended[slot] - copy_begun[slot]);
        wait_us += waited;
        if (waited > wait_max_us) wait_max_us = waited;
        if (++samples == 32) {
            printf("panel_move via=dma2d-async samples=32 copy_us=%" PRIu32 " wait_us=%" PRIu32
                   " wait_max_us=%" PRIu32 "\n", copy_us / 32, wait_us / 32, wait_max_us);
            samples = copy_us = wait_us = wait_max_us = 0;
        }
    }
    return done ? 0 : -1;
#else
    return copy_end(slot) ? 0 : -1;
#endif
}

/* The lanes as the donor draws them. Each deck's waveform is drawn ahead into a ring
   cache, a 1024-pixel-wide picture of the lane's 220 rows whose column `p mod 1024` holds
   track pixel `p`. At each refresh a task above the deck screen's copies the visible
   window from the cache into the shown frame by the 2D-DMA, around the playhead, whose
   columns it never touches. So the lanes move at every refresh, at the position each one
   is scanned out at, however long the deck screen's frame takes; on s195p a frame that
   ran long held both lanes for a refresh and then jumped them twice as far.
   The deck screen draws the cache ahead (`ruzina_lanes_publish`) and keeps its columns in
   memory: it writes them through the PPA, and nothing writes the lanes' rows of the
   frame from the CPU, so neither copy needs its cache lines synced. */
#define LANE_SLOTS 6
#define LINE_US 26               /* one of the panel's 635 lines at 61 refreshes a second */
#define BLANK_US 900             /* the blanking between a refresh's end and line 0 */
typedef struct {
    bool shown;
    int64_t lo, hi;              /* the track pixels the cache holds */
    int64_t anchor_us;           /* when the deck was at `anchor_track_us` (esp_timer) */
    int64_t anchor_track_us;
    int32_t rate_x10000;         /* 10000 at normal speed; 0 when the deck stands */
    int64_t loop_from_us, loop_to_us; /* a running loop, or both 0 */
    uint32_t window_ms;
    uint32_t revision;           /* bumped when shown columns change */
} lane_model_t;
static lane_model_t lane_models[2];
static portMUX_TYPE lane_lock = portMUX_INITIALIZER_UNLOCKED;
static uint16_t *lane_caches[2];
static uint16_t *lane_frame;
static uint32_t lane_x, lane_y[2], lane_w, lane_h, lane_playhead;

static int64_t floor_div(int64_t a, int64_t b) { return a / b - (a % b != 0 && (a < 0) != (b < 0)); }

/* Copies screen columns [s, e) of deck `d`'s lane from its cache, the lane's left edge
   at track pixel `left`, in one or two blocks; returns the next free slot. */
static uint32_t lane_copy(int d, int64_t left, uint32_t s, uint32_t e, uint32_t slot)
{
    while (s < e && slot < LANE_SLOTS) {
        const int64_t c = ((left + s) % 1024 + 1024) % 1024;
        uint32_t n = e - s;
        if (c + n > 1024) n = (uint32_t)(1024 - c);
        if (!copy_begin(slot, lane_caches[d], lane_h, (uint32_t)c, 0, lane_frame, 600,
                        lane_x + s, lane_y[d], n, lane_h)) break;
        slot++;
        s += n;
    }
    return slot;
}

static void lane_task(void *arg)
{
    (void)arg;
    int64_t shown_left[2] = {INT64_MIN, INT64_MIN};
    uint32_t shown_revision[2] = {0, 0};
#if CONFIG_RUZINA_UAC_PROFILE
    uint32_t refreshes_seen = 0, copies = 0, misses = 0, start_max = 0, done_max = 0;
    uint64_t done_total = 0;
    int64_t report_at = esp_timer_get_time() + 20000000;
#endif
    for (;;) {
        ulTaskNotifyTake(pdTRUE, portMAX_DELAY);
        const int64_t now = esp_timer_get_time();
        uint32_t slot = 0;
        for (int d = 0; d < 2; d++) {
            portENTER_CRITICAL(&lane_lock);
            const lane_model_t m = lane_models[d];
            portEXIT_CRITICAL(&lane_lock);
            if (!m.shown || !m.window_ms || m.hi <= m.lo) {
                shown_left[d] = INT64_MIN;
                continue;
            }
            /* Where the deck is when the beam reaches the lane's middle. */
            const int64_t at = now + BLANK_US + (int64_t)(lane_y[d] + lane_h / 2) * LINE_US;
            int64_t track_us = m.anchor_track_us + (at - m.anchor_us) * m.rate_x10000 / 10000;
            if (m.loop_to_us > m.loop_from_us && track_us >= m.loop_to_us)
                track_us = m.loop_from_us + (track_us - m.loop_from_us) % (m.loop_to_us - m.loop_from_us);
            const int64_t left = floor_div(track_us * lane_w, (int64_t)m.window_ms * 1000) - lane_playhead;
            if (left < m.lo || left + lane_w > m.hi) {
#if CONFIG_RUZINA_UAC_PROFILE
                misses++;
#endif
                continue;
            }
            if (left == shown_left[d] && m.revision == shown_revision[d]) continue;
            /* Left of the playhead's three columns, then right of them. */
            slot = lane_copy(d, left, 0, lane_playhead - 1, slot);
            slot = lane_copy(d, left, lane_playhead + 2, lane_w, slot);
            shown_left[d] = left;
            shown_revision[d] = m.revision;
        }
#if CONFIG_RUZINA_UAC_PROFILE
        const uint32_t started = (uint32_t)(esp_timer_get_time() - now);
#endif
        for (uint32_t s = 0; s < slot; s++) copy_end(s);
#if CONFIG_RUZINA_UAC_PROFILE
        /* Profile images: refreshes, the copies made and the deck positions the cache did
           not hold yet, and when the copies were started and done after the refresh. */
        refreshes_seen++;
        if (slot) {
            const uint32_t done = (uint32_t)(esp_timer_get_time() - now);
            copies++;
            done_total += done;
            if (done > done_max) done_max = done;
            if (started > start_max) start_max = started;
        }
        if (now >= report_at) {
            printf("lanes refreshes=%" PRIu32 " copied=%" PRIu32 " misses=%" PRIu32 " start_max_us=%" PRIu32
                   " done_mean_us=%" PRIu32 " done_max_us=%" PRIu32 "\n", refreshes_seen, copies, misses,
                   start_max, copies ? (uint32_t)(done_total / copies) : 0, done_max);
            refreshes_seen = copies = misses = start_max = done_max = 0;
            done_total = 0;
            report_at = now + 20000000;
        }
#endif
    }
}

/* Starts the lanes: deck `d`'s lane is `w` by `h` pixels at (x, y[d]) of `frame`, the shown
   frame buffer, with its playhead in columns playhead - 1 to playhead + 1. Makes each deck's
   cache and returns them in `caches`; false if they or the task cannot be had. */
bool ruzina_lanes_start(uint16_t *frame, uint32_t x, uint32_t y0, uint32_t y1, uint32_t w, uint32_t h,
                        uint32_t playhead, uint16_t **caches)
{
    if (lane_task_handle) return false;
    if (w > 1024 || playhead < 1 || playhead + 2 > w || !copy_ready()) return false;
    for (int d = 0; d < 2; d++) {
        lane_caches[d] = heap_caps_aligned_calloc(64, 1024 * h, 2, MALLOC_CAP_SPIRAM);
        if (!lane_caches[d]) return false;
        caches[d] = lane_caches[d];
    }
    lane_frame = frame;
    lane_x = x;
    lane_y[0] = y0;
    lane_y[1] = y1;
    lane_w = w;
    lane_h = h;
    lane_playhead = playhead;
    /* On the deck screen's core, above it (4) and the SD client (3). */
    return xTaskCreatePinnedToCore(lane_task, "lanes", 4096, NULL, 6, &lane_task_handle, 1) == pdPASS;
}

/* Deck `d`'s lane from now on: whether it is shown, the track pixels its cache holds, and
   where the deck is: at `track_us` of the track at esp_timer `at_us`, going at
   `rate_x10000` / 10000 of normal speed, in a loop from `loop_from_us` to `loop_to_us` (both
   0 for none). `revision` changes when columns already shown were drawn again. */
void ruzina_lanes_publish(uint32_t d, bool shown, int64_t lo, int64_t hi, int64_t at_us, int64_t track_us,
                          int32_t rate_x10000, int64_t loop_from_us, int64_t loop_to_us, uint32_t window_ms,
                          uint32_t revision)
{
    if (d > 1) return;
    portENTER_CRITICAL(&lane_lock);
    lane_models[d] = (lane_model_t){
        .shown = shown, .lo = lo, .hi = hi, .anchor_us = at_us, .anchor_track_us = track_us,
        .rate_x10000 = rate_x10000, .loop_from_us = loop_from_us, .loop_to_us = loop_to_us,
        .window_ms = window_ms, .revision = revision,
    };
    portEXIT_CRITICAL(&lane_lock);
}

/* Copies a w by h block of pixels from column `from_x` of `from`, `from_w` to a row, to
   column x of a lane cache (1024 by h), by the PPA. */
int32_t ruzina_lanes_blit(const uint16_t *from, uint32_t from_w, uint32_t from_x, uint16_t *cache,
                          uint32_t x, uint32_t w, uint32_t h)
{
    if (!w || !h || from_x + w > from_w || x + w > 1024 || h != lane_h || !mover_ready()) return -1;
    ppa_srm_oper_config_t blit = {
        .in = {.buffer = from, .pic_w = from_w, .pic_h = h, .block_w = w, .block_h = h,
               .block_offset_x = from_x, .block_offset_y = 0, .srm_cm = PPA_SRM_COLOR_MODE_RGB565},
        .out = {.buffer = cache, .buffer_size = 1024 * h * 2, .pic_w = 1024, .pic_h = h,
                .block_offset_x = x, .block_offset_y = 0, .srm_cm = PPA_SRM_COLOR_MODE_RGB565},
        .rotation_angle = PPA_SRM_ROTATION_ANGLE_0, .scale_x = 1.0f, .scale_y = 1.0f,
        .mode = PPA_TRANS_MODE_BLOCKING,
    };
    return ppa_do_scale_rotate_mirror(mover, &blit) == ESP_OK ? 0 : -1;
}

/* A strip of `pixels` pixels in internal RAM, made on the first call, or NULL. The deck
   screen draws there, Slint's lines and a lane's columns, and the PPA copies them into a
   frame buffer, so the CPU's writes stay off PSRAM: there they slow the USB audio fill on
   the other core, the PPA's copies hardly at all (the s165 boot bench). */
uint16_t *ruzina_panel_strip(uint32_t pixels)
{
    static uint16_t *strip;
    static uint32_t made;
    if (!strip && pixels) {
        strip = heap_caps_aligned_calloc(64, pixels, 2, MALLOC_CAP_INTERNAL | MALLOC_CAP_DMA);
        made = strip ? pixels : 0;
    }
    return made == pixels ? strip : NULL;
}

/* Copies the `w` by `h` pixels at column `from_x` of `from`, `from_w` to a row, to (x, y)
   of frame buffer `to`, in hardware. As for ruzina_panel_move, the PPA driver invalidates
   the rows of `to`, so the caller writes them back before. */
int32_t ruzina_panel_blit(const uint16_t *from, uint32_t from_w, uint32_t from_x, uint16_t *to,
                          uint32_t x, uint32_t y, uint32_t w, uint32_t h)
{
    if (!w || !h || from_x + w > from_w || x + w > 1024 || y + h > 600 || !mover_ready()) return -1;
    ppa_srm_oper_config_t blit = {
        .in = {.buffer = from, .pic_w = from_w, .pic_h = h, .block_w = w, .block_h = h,
               .block_offset_x = from_x, .block_offset_y = 0, .srm_cm = PPA_SRM_COLOR_MODE_RGB565},
        .out = {.buffer = to, .buffer_size = 1024 * 600 * 2, .pic_w = 1024, .pic_h = 600,
                .block_offset_x = x, .block_offset_y = y, .srm_cm = PPA_SRM_COLOR_MODE_RGB565},
        .rotation_angle = PPA_SRM_ROTATION_ANGLE_0, .scale_x = 1.0f, .scale_y = 1.0f,
        .mode = PPA_TRANS_MODE_BLOCKING,
    };
    return ppa_do_scale_rotate_mirror(mover, &blit) == ESP_OK ? 0 : -1;
}

/* Waits for the end of the next refresh. */
void ruzina_panel_wait_refresh(void)
{
    xSemaphoreTake(refreshed, 0);
    xSemaphoreTake(refreshed, pdMS_TO_TICKS(50));
}

/* Shows `buffer` from the next refresh on and waits until it is on the panel, so the
   other buffer is free to draw into. The caller wrote its changed rows back from cache. */
void ruzina_panel_present(uint16_t *buffer)
{
    xSemaphoreTake(refreshed, 0);
    esp_lcd_panel_draw_bitmap(panel, 0, 0, 1, 1, buffer);
    xSemaphoreTake(refreshed, pdMS_TO_TICKS(50));
}
#endif

static esp_err_t brightness(unsigned duty)
{
    esp_err_t err = ledc_set_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_0, duty);
    return err == ESP_OK ? ledc_update_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_0) : err;
}

/* The deck's brightness setting, 10 to 100 percent of the 10-bit PWM. Before the panel
 * is up it is kept for the panel's start, so a failed panel stays dark. */
static bool lit;
static uint32_t backlight_percent = 25;
static unsigned backlight_duty(void) { return backlight_percent * 1023u / 100u; }
void ruzina_display_backlight(uint32_t percent)
{
    if (percent < 10) percent = 10;
    if (percent > 100) percent = 100;
    backlight_percent = percent;
    if (lit) brightness(backlight_duty());
}

/* Brings up the New_Panel with one driver-owned RGB565 framebuffer. `fill`
 * paints it before scanout starts; the backlight stays off until then. On
 * failure the backlight is off and handles are retained until app restart. */
esp_err_t ruzina_panel_start(void (*fill)(uint16_t *frame), uint16_t **frame_out)
{
    esp_err_t err;
#define TRY(call) do { err = (call); if (err != ESP_OK) goto fail; } while (0)
    ledc_timer_config_t timer = {.speed_mode=LEDC_LOW_SPEED_MODE, .duty_resolution=LEDC_TIMER_10_BIT,
        .timer_num=LEDC_TIMER_1, .freq_hz=20000, .clk_cfg=LEDC_AUTO_CLK};
    TRY(ledc_timer_config(&timer));
    ledc_channel_config_t pwm = {.gpio_num=23, .speed_mode=LEDC_LOW_SPEED_MODE,
        .channel=LEDC_CHANNEL_0, .timer_sel=LEDC_TIMER_1, .duty=0};
    TRY(ledc_channel_config(&pwm)); pwm_ready = true;
    esp_ldo_channel_config_t power = {.chan_id=3, .voltage_mv=2500};
    TRY(esp_ldo_acquire_channel(&power, &phy_power));
    esp_lcd_dsi_bus_config_t dsi = {.bus_id=0, .num_data_lanes=2, .phy_clk_src=MIPI_DSI_PHY_CLK_SRC_DEFAULT,
        .lane_bit_rate_mbps=750};
    TRY(esp_lcd_new_dsi_bus(&dsi, &bus));
    esp_lcd_dbi_io_config_t dbi = {.virtual_channel=0, .lcd_cmd_bits=8, .lcd_param_bits=8};
    TRY(esp_lcd_new_panel_io_dbi(bus, &dbi, &io));
    gpio_config_t reset = {.pin_bit_mask=1ULL << 0, .mode=GPIO_MODE_OUTPUT};
    TRY(gpio_config(&reset));
    TRY(gpio_set_level(0, 1)); vTaskDelay(pdMS_TO_TICKS(10));
    TRY(gpio_set_level(0, 0)); vTaskDelay(pdMS_TO_TICKS(10));
    TRY(gpio_set_level(0, 1)); vTaskDelay(pdMS_TO_TICKS(120));
    for (unsigned i=0; i<sizeof(new_panel_commands)/sizeof(new_panel_commands[0]); ++i) {
        const panel_command_t *cmd = &new_panel_commands[i];
        TRY(esp_lcd_panel_io_tx_param(io, cmd->command, cmd->data, cmd->length));
        if (cmd->delay_ms) vTaskDelay(pdMS_TO_TICKS(cmd->delay_ms));
    }
    esp_lcd_dpi_panel_config_t dpi = {.virtual_channel=0, .dpi_clk_src=MIPI_DSI_DPI_CLK_SRC_DEFAULT,
        .dpi_clock_freq_mhz=52, .in_color_format=LCD_COLOR_FMT_RGB565,
        .out_color_format=LCD_COLOR_FMT_RGB565,
#if CONFIG_RUZINA_SLINT_UI
        .num_fbs=2,
#else
        .num_fbs=1,
#endif
        .video_timing={.h_size=1024,.v_size=600,.hsync_back_porch=136,.hsync_pulse_width=24,
            .hsync_front_porch=160,.vsync_back_porch=21,.vsync_pulse_width=2,.vsync_front_porch=12}};
    TRY(esp_lcd_new_panel_dpi(bus, &dpi, &panel));
    void *frame_storage = NULL;
#if CONFIG_RUZINA_SLINT_UI
    void *back_storage = NULL;
    TRY(esp_lcd_dpi_panel_get_frame_buffer(panel, 2, &frame_storage, &back_storage));
    back_frame = back_storage;
    fill(back_frame);
    TRY(esp_cache_msync(back_frame, 1024*600*2, ESP_CACHE_MSYNC_FLAG_DIR_C2M));
    refreshed = xSemaphoreCreateBinary();
    if (!refreshed) { err = ESP_ERR_NO_MEM; goto fail; }
    esp_lcd_dpi_panel_event_callbacks_t callbacks = {.on_frame_buf_complete = frame_done};
    TRY(esp_lcd_dpi_panel_register_event_callbacks(panel, &callbacks, NULL));
#else
    TRY(esp_lcd_dpi_panel_get_frame_buffer(panel, 1, &frame_storage));
#endif
    frame = frame_storage;
    fill(frame);
    TRY(esp_cache_msync(frame, 1024*600*2, ESP_CACHE_MSYNC_FLAG_DIR_C2M));
    TRY(esp_lcd_panel_init(panel));
    TRY(brightness(backlight_duty()));
    lit = true;
    printf("S6_DISPLAY_INIT commands=60 lanes=2 mbps=750 pixel_mhz=52 size=1024x600 format=RGB565 framebuffer_bytes=1228800 backlight_percent=%" PRIu32 "\n", backlight_percent);
    *frame_out = frame;
    return ESP_OK;
fail:
    printf("s6_error=%s backlight=OFF\n", esp_err_to_name(err));
    if (pwm_ready) brightness(0);
    return err;
#undef TRY
}

/* GT911 polling over I2C0 (GPIO7/8); the reset and IRQ lines stay untouched. */
static i2c_master_bus_handle_t i2c;
static i2c_master_dev_handle_t touch;
static uint16_t touch_width, touch_height;

static esp_err_t read_touch(uint16_t reg, uint8_t *data, size_t size)
{
    uint8_t address[2] = {reg >> 8, reg & 255};
    return i2c_master_transmit_receive(touch, address, 2, data, size, 100);
}

/* Finds the controller, checks its ID and reads the current RAM config size. */
esp_err_t ruzina_touch_start(void)
{
    esp_err_t err;
#define TRY(call) do { err = (call); if (err != ESP_OK) return err; } while (0)
    i2c_master_bus_config_t wire = {.i2c_port=0, .sda_io_num=7, .scl_io_num=8,
        .clk_source=I2C_CLK_SRC_DEFAULT, .glitch_ignore_cnt=7, .flags.enable_internal_pullup=true};
    TRY(i2c_new_master_bus(&wire, &i2c));
    uint16_t address = 0;
    if (i2c_master_probe(i2c, 0x5d, 100) == ESP_OK) address=0x5d;
    else if (i2c_master_probe(i2c, 0x14, 100) == ESP_OK) address=0x14;
    if (!address) return ESP_ERR_NOT_FOUND;
    i2c_device_config_t dev = {.dev_addr_length=I2C_ADDR_BIT_LEN_7, .device_address=address, .scl_speed_hz=100000};
    TRY(i2c_master_bus_add_device(i2c, &dev, &touch));
    uint8_t id[4], dimensions[4];
    TRY(read_touch(0x8140, id, sizeof(id)));
    if (memcmp(id, "911", 3) != 0) return ESP_ERR_INVALID_RESPONSE;
    TRY(read_touch(0x8048, dimensions, sizeof(dimensions)));
    touch_width=dimensions[0] | dimensions[1]<<8; touch_height=dimensions[2] | dimensions[3]<<8;
    if (touch_width<2 || touch_height<2 || touch_width>4096 || touch_height>4096) return ESP_ERR_INVALID_RESPONSE;
    printf("S6_TOUCH_ID address=0x%02x id=%02x%02x%02x%02x config_size=%ux%u mode=POLLING reset_irq=UNTOUCHED\n",
           address,id[0],id[1],id[2],id[3],touch_width,touch_height);
    return ESP_OK;
#undef TRY
}

/* One poll: returns the reported point count (0..5) with the first point in
 * raw and panel coordinates, and adds failed transfers to `errors`. */
unsigned ruzina_touch_poll(unsigned *raw_x, unsigned *raw_y, unsigned *x, unsigned *y, uint32_t *errors)
{
    if (!touch) return 0; /* not started, or its start failed */
    uint8_t status;
    unsigned count = 0;
    esp_err_t err = read_touch(0x814e, &status, 1);
    if (err == ESP_OK && (status & 0x80)) {
        count = status & 15;
        uint8_t points[40];
        if (count > 5) { (*errors)++; count = 0; }
        if (count && read_touch(0x814f, points, count * 8) == ESP_OK) {
            *raw_x = points[1] | points[2] << 8;
            *raw_y = points[3] | points[4] << 8;
            /* Input dimensions come from the chip's current RAM config,
             * never assumed from a stale 800x480 BSP calibration message. */
            *x = *raw_x * 1023u / (touch_width - 1u);
            *y = *raw_y * 599u / (touch_height - 1u);
            if (*x > 1023) *x = 1023;
            if (*y > 599) *y = 599;
        } else if (count) { (*errors)++; count = 0; }
        /* Acknowledge only the ready status byte, never persistent config. */
        const uint8_t clear[] = {0x81, 0x4e, 0};
        if (i2c_master_transmit(touch, clear, sizeof(clear), 100) != ESP_OK) (*errors)++;
    } else if (err != ESP_OK) (*errors)++;
    return count;
}

#if CONFIG_RUZINA_DISPLAY_TOUCH
static void touch_task(void *unused)
{
    (void)unused;
    uint32_t samples = 0, errors = 0;
    int64_t report = 0;
    for (;;) {
        unsigned x, y, mapped_x, mapped_y;
        unsigned count = ruzina_touch_poll(&x, &y, &mapped_x, &mapped_y, &errors);
        if (count)
            printf("touch_sample=%" PRIu32 " count=%u raw=%u,%u mapped=%u,%u\n",
                   ++samples, count, x, y, mapped_x, mapped_y);
        if (esp_timer_get_time() - report > 5000000) {
            printf("touch_health samples=%" PRIu32 " errors=%" PRIu32 "\n", samples, errors);
            report = esp_timer_get_time();
        }
        vTaskDelay(pdMS_TO_TICKS(40));
    }
}

/* Fill exactly one driver-owned framebuffer before scanout starts. No
 * active-frame CPU writes, alternate owner, or source DMA buffer lifetime. */
static void stripes(uint16_t *fb)
{
    const uint16_t colors[] = {0xf800,0x07e0,0x001f,0xffff,0x0000,0xffe0,0x07ff,0xf81f};
    for (unsigned y=0; y<600; ++y) {
        for (unsigned x=0; x<1024; ++x) {
            bool marker=(x<32&&y<32)||(x>=992&&y<32)||(x<32&&y>=568)||(x>=992&&y>=568);
            fb[y*1024+x] = marker ? 0xffff : colors[x/128];
        }
    }
}

esp_err_t ruzina_display_touch_start(void)
{
    esp_err_t err;
#define TRY(call) do { err = (call); if (err != ESP_OK) goto fail; } while (0)
    uint16_t *fb;
    err = ruzina_panel_start(stripes, &fb);
    if (err != ESP_OK) return err;

    TRY(ruzina_touch_start());
    if (xTaskCreate(touch_task,"s6_touch",4096,NULL,1,NULL)!=pdPASS) {err=ESP_ERR_NO_MEM;goto fail;}
    printf("s6_readiness=AWAITING_OPERATOR_COLORS_CORNERS_AND_TOUCH\n");
    return ESP_OK;
fail:
    printf("s6_error=%s backlight=OFF\n", esp_err_to_name(err));
    if (pwm_ready) brightness(0);
    /* Retain failed peripheral handles for diagnostics until controlled app
     * restart. No concurrent task exists on these initialization-failure paths. */
    return err;
#undef TRY
}
#endif
