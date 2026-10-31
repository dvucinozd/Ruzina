#include <inttypes.h>
#include <stdio.h>
#include <string.h>
#include "esp_cache.h"
#include "esp_err.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "font24.h"

/* Status screen for the sd-rust-load profile. The SD client, owner and touch
 * task only copy scalars and bounded bytes into `shared` under a short
 * critical section; the UI task alone draws into the scanned-out framebuffer,
 * row by row, and writes each changed row back from cache. */
esp_err_t ruzina_panel_start(void (*fill)(uint16_t *frame), uint16_t **frame_out);
esp_err_t ruzina_touch_start(void);
unsigned ruzina_touch_poll(unsigned *raw_x, unsigned *raw_y, unsigned *x, unsigned *y, uint32_t *errors);
/* Rust: asks the load task to preflight a track out of turn and report it. */
extern void ruzina_legacy_load_select(uint32_t index);

#define WIDTH 1024
#define HEIGHT 600
#define COLS (WIDTH / 2 / FONT24_WIDTH)
#define BANDS (HEIGHT / FONT24_HEIGHT)
#define TRACK_TOP 4
#define DETAIL_BANDS 4
#define TRACK_ROWS (BANDS - TRACK_TOP - DETAIL_BANDS)
#define DETAIL_TOP (TRACK_TOP + TRACK_ROWS)
#define MAX_TRACKS (2 * TRACK_ROWS)
#define TEXT_BYTES 96
#define PEAKS 400
#define WAVE_LEFT 12
#define WAVE_WIDTH 1000

enum { BACKGROUND = 0x0000, FOREGROUND = 0xffff, DIM = 0x8410, ACCENT = 0xfd20,
       OK_COLOR = 0x07e0, NONE_COLOR = 0xffe0, ERROR_COLOR = 0xf800,
       SELECTED = 0x2124, WAVE = 0x5d1f, WAVE_PEAK = 0xffff };

typedef struct {
    uint32_t index, bpm_x100, duration_ms, beats, cues, memory, peaks_len;
    char key[16];
    uint8_t peaks[PEAKS];
} detail_t;

typedef struct {
    uint32_t version, pass, tracks, playlists, loaded, psram_free, psram_min, stack_free;
    bool failed;
    uint32_t failure;       /* stage << 8 | code */
    char note[TEXT_BYTES];  /* owner state */
    int32_t selected;       /* -1 none */
    int32_t deck_track[2];  /* track loaded on deck 1/2 by the controller; -1 none */
    uint32_t detail_version;
    detail_t detail;
    int8_t outcome[MAX_TRACKS];
    char title[MAX_TRACKS][TEXT_BYTES];
} status_t;

static portMUX_TYPE lock = portMUX_INITIALIZER_UNLOCKED;
static status_t shared, drawn;
static uint16_t *frame;
static bool ready;

static void dark(uint16_t *fb) { memset(fb, 0, WIDTH * HEIGHT * 2); }

static unsigned glyph(uint32_t cp)
{
    unsigned lo = 0, hi = FONT24_GLYPHS;
    while (lo < hi) {
        unsigned mid = (lo + hi) / 2;
        if (font24_points[mid] == cp) return mid;
        if (font24_points[mid] < cp) lo = mid + 1; else hi = mid;
    }
    return glyph('?');
}

/* Decodes one UTF-8 scalar; malformed input yields '?' and advances a byte. */
static uint32_t next(const char **s)
{
    const uint8_t *p = (const uint8_t *)*s;
    uint32_t cp = '?';
    unsigned n = 1;
    if (p[0] < 0x80) cp = p[0];
    else if ((p[0] & 0xe0) == 0xc0 && (p[1] & 0xc0) == 0x80) { cp = (p[0] & 31u) << 6 | (p[1] & 63u); n = 2; }
    else if ((p[0] & 0xf0) == 0xe0 && (p[1] & 0xc0) == 0x80 && (p[2] & 0xc0) == 0x80) {
        cp = (p[0] & 15u) << 12 | (p[1] & 63u) << 6 | (p[2] & 63u); n = 3;
    }
    *s += n;
    return cp;
}

static void text(unsigned x, unsigned y, unsigned cols, uint16_t color, const char *s)
{
    for (unsigned c = 0; c < cols && *s; ++c, x += FONT24_WIDTH) {
        const uint8_t *bits = font24_bits[glyph(next(&s))];
        for (unsigned row = 0; row < FONT24_HEIGHT; ++row) {
            unsigned line = bits[2 * row] << 8 | bits[2 * row + 1];
            uint16_t *px = frame + (y + row) * WIDTH + x;
            for (unsigned col = 0; col < FONT24_WIDTH; ++col)
                if (line & (0x8000u >> col)) px[col] = color;
        }
    }
}

static void fill(unsigned x, unsigned y, unsigned w, unsigned h, uint16_t color)
{
    for (unsigned row = y; row < y + h; ++row)
        for (unsigned col = x; col < x + w; ++col) frame[row * WIDTH + col] = color;
}

/* Clears, draws and writes back one full-width 24-pixel row band. */
static void band(unsigned row, void (*paint)(unsigned y, const status_t *), const status_t *s)
{
    unsigned y = row * FONT24_HEIGHT;
    uint16_t *start = frame + y * WIDTH;
    memset(start, 0, WIDTH * FONT24_HEIGHT * 2);
    paint(y, s);
    esp_cache_msync(start, WIDTH * FONT24_HEIGHT * 2, ESP_CACHE_MSYNC_FLAG_DIR_C2M);
}

/* Before any controller load: the profile banner. After one: what each deck
 * holds, as chosen with the controller's Load buttons. */
static void paint_header(unsigned y, const status_t *s)
{
    if (s->deck_track[0] < 0 && s->deck_track[1] < 0) {
        text(12, y, 80, ACCENT, "RUZINA  sd-rust-load  JC1060 v1.3  read-only SD  tap or browse a track");
        return;
    }
    for (unsigned deck = 0; deck < 2; ++deck) {
        unsigned x = deck * (WIDTH / 2) + 12;
        int32_t track = s->deck_track[deck];
        text(x, y, 3, ACCENT, deck ? "D2" : "D1");
        text(x + 36, y, COLS - 4, track >= 0 ? FOREGROUND : DIM, track >= 0 ? s->title[track] : "empty");
    }
}

static void paint_counts(unsigned y, const status_t *s)
{
    char line[2 * TEXT_BYTES];
    snprintf(line, sizeof(line), "Pass %" PRIu32 "   Tracks %" PRIu32 "   Analysis %" PRIu32 "/%" PRIu32
             "   Playlists %" PRIu32, s->pass, s->tracks, s->loaded, s->tracks, s->playlists);
    text(12, y, 84, FOREGROUND, line);
}

static void paint_memory(unsigned y, const status_t *s)
{
    char line[2 * TEXT_BYTES];
    if (s->failed)
        snprintf(line, sizeof(line), "FAILED stage %" PRIu32 " code %" PRIu32 "   %s",
                 s->failure >> 8, s->failure & 255, s->note);
    else
        snprintf(line, sizeof(line), "PSRAM free %" PRIu32 " KiB  min %" PRIu32 " KiB   stack %" PRIu32 " B   %s",
                 s->psram_free / 1024, s->psram_min / 1024, s->stack_free, s->note);
    text(12, y, 84, s->failed ? ERROR_COLOR : DIM, line);
}

static void paint_tracks(unsigned y, const status_t *s)
{
    unsigned row = y / FONT24_HEIGHT - TRACK_TOP;
    for (unsigned half = 0; half < 2; ++half) {
        unsigned index = half * TRACK_ROWS + row, x = half * (WIDTH / 2) + 12;
        if (index >= s->tracks || index >= MAX_TRACKS) continue;
        if ((int32_t)index == s->selected) fill(half * (WIDTH / 2), y, WIDTH / 2, FONT24_HEIGHT, SELECTED);
        static const uint16_t marks[] = {OK_COLOR, NONE_COLOR, ERROR_COLOR, ERROR_COLOR};
        int8_t outcome = s->outcome[index];
        fill(x, y + 6, 12, 12, outcome < 0 ? DIM : marks[outcome & 3]);
        char label[8];
        snprintf(label, sizeof(label), "%2u", index + 1);
        text(x + 18, y, 2, DIM, label);
        text(x + 54, y, COLS - 6, s->title[index][0] ? FOREGROUND : DIM,
             s->title[index][0] ? s->title[index] : "...");
    }
}

static bool detail_ready(const status_t *s)
{
    return s->selected >= 0 && s->detail_version && s->detail.index == (uint32_t)s->selected;
}

static void paint_selected(unsigned y, const status_t *s)
{
    char line[2 * TEXT_BYTES];
    if (s->selected < 0) {
        text(12, y, 84, DIM, "Tap a track to show its analysis");
        return;
    }
    snprintf(line, sizeof(line), "%2" PRId32 "  %s", s->selected + 1, s->title[s->selected]);
    text(12, y, 84, ACCENT, line);
}

static void paint_facts(unsigned y, const status_t *s)
{
    char line[2 * TEXT_BYTES];
    if (s->selected < 0) return;
    if (!detail_ready(s)) {
        text(12, y, 84, DIM, "Loading analysis...");
        return;
    }
    const detail_t *d = &s->detail;
    unsigned seconds = d->duration_ms / 1000;
    snprintf(line, sizeof(line), "BPM %" PRIu32 ".%02" PRIu32 "   Key %s   %u:%02u   Beats %" PRIu32
             "   Hot cues %" PRIu32 "   Memory %" PRIu32 "%s",
             d->bpm_x100 / 100, d->bpm_x100 % 100, d->key[0] ? d->key : "-", seconds / 60, seconds % 60,
             d->beats, d->cues, d->memory, d->peaks_len ? "" : "   (no analysis)");
    text(12, y, 84, FOREGROUND, line);
}

/* The preview is mirrored around the line between the last two bands. */
static void paint_wave(unsigned y, const status_t *s, bool upper)
{
    if (!detail_ready(s) || !s->detail.peaks_len) return;
    const detail_t *d = &s->detail;
    for (unsigned x = 0; x < WAVE_WIDTH; ++x) {
        unsigned peak = d->peaks[x * d->peaks_len / WAVE_WIDTH];
        unsigned height = peak * (FONT24_HEIGHT - 1) / 255;
        uint16_t color = peak > 220 ? WAVE_PEAK : WAVE;
        for (unsigned h = 0; h <= height; ++h) {
            unsigned py = upper ? y + FONT24_HEIGHT - 1 - h : y + h;
            frame[py * WIDTH + WAVE_LEFT + x] = color;
        }
    }
}
static void paint_wave_upper(unsigned y, const status_t *s) { paint_wave(y, s, true); }
static void paint_wave_lower(unsigned y, const status_t *s) { paint_wave(y, s, false); }

static bool row_dirty(unsigned row, const status_t *now)
{
    for (unsigned half = 0; half < 2; ++half) {
        unsigned i = half * TRACK_ROWS + row;
        bool was = drawn.selected == (int32_t)i, is = now->selected == (int32_t)i;
        if (was != is || now->outcome[i] != drawn.outcome[i] || strcmp(now->title[i], drawn.title[i]))
            return true;
    }
    return false;
}

static void ui_task(void *unused)
{
    (void)unused;
    static status_t now;
    drawn.version = UINT32_MAX;
    drawn.selected = -1;
    for (;;) {
        vTaskDelay(pdMS_TO_TICKS(100));
        portENTER_CRITICAL(&lock);
        bool changed = shared.version != drawn.version;
        if (changed) now = shared;
        portEXIT_CRITICAL(&lock);
        if (!changed) continue;
        bool first = drawn.version == UINT32_MAX;
        if (first || now.deck_track[0] != drawn.deck_track[0] || now.deck_track[1] != drawn.deck_track[1])
            band(0, paint_header, &now);
        if (first || now.pass != drawn.pass || now.tracks != drawn.tracks || now.loaded != drawn.loaded
                || now.playlists != drawn.playlists)
            band(1, paint_counts, &now);
        if (first || now.psram_free != drawn.psram_free || now.psram_min != drawn.psram_min
                || now.stack_free != drawn.stack_free || now.failed != drawn.failed
                || strcmp(now.note, drawn.note))
            band(2, paint_memory, &now);
        for (unsigned row = 0; row < TRACK_ROWS; ++row)
            if (first || now.tracks != drawn.tracks || row_dirty(row, &now))
                band(TRACK_TOP + row, paint_tracks, &now);
        bool selection = first || now.selected != drawn.selected
            || (now.selected >= 0 && strcmp(now.title[now.selected], drawn.title[now.selected]));
        bool detail = selection || now.detail_version != drawn.detail_version;
        if (selection) band(DETAIL_TOP, paint_selected, &now);
        if (detail) {
            band(DETAIL_TOP + 1, paint_facts, &now);
            band(DETAIL_TOP + 2, paint_wave_upper, &now);
            band(DETAIL_TOP + 3, paint_wave_lower, &now);
        }
        drawn = now;
    }
}

/* Maps a press on a track row to its catalog index, or -1. */
static int32_t hit(unsigned x, unsigned y, uint32_t tracks)
{
    unsigned row = y / FONT24_HEIGHT;
    if (row < TRACK_TOP || row >= DETAIL_TOP) return -1;
    unsigned index = (x >= WIDTH / 2 ? TRACK_ROWS : 0) + row - TRACK_TOP;
    return index < tracks && index < MAX_TRACKS ? (int32_t)index : -1;
}

static void touch_task(void *unused)
{
    (void)unused;
    uint32_t errors = 0, presses = 0;
    bool down = false;
    for (;;) {
        unsigned raw_x, raw_y, x, y;
        bool pressed = ruzina_touch_poll(&raw_x, &raw_y, &x, &y, &errors) > 0;
        if (pressed && !down) {
            portENTER_CRITICAL(&lock);
            int32_t index = hit(x, y, shared.tracks);
            if (index >= 0) { shared.selected = index; shared.version++; }
            portEXIT_CRITICAL(&lock);
            printf("load_touch press=%" PRIu32 " mapped=%u,%u index=%" PRId32 " errors=%" PRIu32 "\n",
                   ++presses, x, y, index, errors);
            if (index >= 0) ruzina_legacy_load_select((uint32_t)index);
        }
        down = pressed;
        vTaskDelay(pdMS_TO_TICKS(40));
    }
}

/* Controller browse: move the selection (clamped to the catalog) and ask the
 * load task for its detail, exactly as a tap does. Runs on the USB task. */
void ruzina_load_display_browse(int32_t delta)
{
    if (!ready) return;
    portENTER_CRITICAL(&lock);
    int32_t index = -1;
    if (shared.tracks) {
        int32_t last = (int32_t)(shared.tracks < MAX_TRACKS ? shared.tracks : MAX_TRACKS) - 1;
        index = shared.selected < 0 ? 0 : shared.selected + delta;
        if (index < 0) index = 0;
        if (index > last) index = last;
        shared.selected = index;
        shared.version++;
    }
    portEXIT_CRITICAL(&lock);
    if (index >= 0) ruzina_legacy_load_select((uint32_t)index);
}

/* Browse press: re-request the selected track's detail. */
void ruzina_load_display_choose(void)
{
    if (!ready) return;
    portENTER_CRITICAL(&lock);
    int32_t index = shared.selected;
    portEXIT_CRITICAL(&lock);
    if (index >= 0) ruzina_legacy_load_select((uint32_t)index);
}

/* Load button: assign the selected track to deck 1 or 2 in the header. */
void ruzina_load_display_load(uint32_t deck)
{
    if (!ready || deck < 1 || deck > 2) return;
    portENTER_CRITICAL(&lock);
    if (shared.selected >= 0) {
        shared.deck_track[deck - 1] = shared.selected;
        shared.version++;
    }
    portEXIT_CRITICAL(&lock);
}

esp_err_t ruzina_load_display_start(void)
{
    memset(shared.outcome, -1, sizeof(shared.outcome));
    shared.selected = -1;
    shared.deck_track[0] = shared.deck_track[1] = -1;
    esp_err_t err = ruzina_panel_start(dark, &frame);
    if (err != ESP_OK) return err;
    if (xTaskCreatePinnedToCore(ui_task, "load_ui", 4096, NULL, 1, NULL, 0) != pdPASS) return ESP_ERR_NO_MEM;
    ready = true;
    printf("S6_LOAD_DISPLAY rows=%u columns=2 font=terminus12x24 max_tracks=%u detail_bands=%u\n",
           TRACK_ROWS, MAX_TRACKS, DETAIL_BANDS);
    /* The screen still reports status when touch is absent. */
    err = ruzina_touch_start();
    if (err != ESP_OK) printf("load_touch=INIT_FAILED error=%s\n", esp_err_to_name(err));
    else if (xTaskCreatePinnedToCore(touch_task, "load_touch", 3072, NULL, 1, NULL, 0) != pdPASS)
        printf("load_touch=TASK_FAILED\n");
    return ESP_OK;
}

/* Load telemetry events (see sd_bridge.c): 1 mounted, 2 catalog, 3 playlists,
 * 4 track index<<8|outcome, 5 pass, 6 failure stage<<8|code. */
void ruzina_load_display_event(uint32_t pass, uint32_t event, uint32_t value, uint32_t stack_free,
                               uint32_t psram_free, uint32_t psram_min)
{
    if (!ready) return;
    char note[TEXT_BYTES];
    if (event == 1) snprintf(note, sizeof(note), "SD mounted LBA %" PRIu32, value);
    portENTER_CRITICAL(&lock);
    shared.pass = pass; shared.stack_free = stack_free;
    shared.psram_free = psram_free; shared.psram_min = psram_min;
    switch (event) {
        case 1: memcpy(shared.note, note, sizeof(note)); break;
        case 2: shared.tracks = value; break;
        case 3: shared.playlists = value; break;
        case 4: if ((value >> 8) < MAX_TRACKS) shared.outcome[value >> 8] = (int8_t)(value & 3); break;
        case 5: shared.loaded = value; break;
        case 6: shared.failed = true; shared.failure = value; break;
        default: break;
    }
    shared.version++;
    portEXIT_CRITICAL(&lock);
}

/* Copies at most TEXT_BYTES-1 bytes; the caller's buffer is not retained. */
void ruzina_load_display_title(uint32_t index, const uint8_t *artist, uint32_t artist_len,
                               const uint8_t *title, uint32_t title_len)
{
    if (!ready || index >= MAX_TRACKS) return;
    char line[TEXT_BYTES];
    size_t n = 0;
    for (uint32_t i = 0; i < artist_len && n < sizeof(line) - 1; ++i) line[n++] = (char)artist[i];
    if (artist_len && n < sizeof(line) - 3) { line[n++] = ' '; line[n++] = '-'; line[n++] = ' '; }
    for (uint32_t i = 0; i < title_len && n < sizeof(line) - 1; ++i) line[n++] = (char)title[i];
    line[n] = 0;
    portENTER_CRITICAL(&lock);
    memcpy(shared.title[index], line, n + 1);
    shared.version++;
    portEXIT_CRITICAL(&lock);
}

/* Copies the selected track's detail; key and peaks are not retained. */
void ruzina_load_display_detail(uint32_t index, uint32_t bpm_x100, uint32_t duration_ms, const uint8_t *key,
                                uint32_t key_len, uint32_t beats, uint32_t cues, uint32_t memory,
                                const uint8_t *peaks, uint32_t peaks_len)
{
    if (!ready) return;
    static detail_t d;
    d.index = index; d.bpm_x100 = bpm_x100; d.duration_ms = duration_ms;
    d.beats = beats; d.cues = cues; d.memory = memory;
    d.peaks_len = peaks_len < PEAKS ? peaks_len : PEAKS;
    size_t n = key_len < sizeof(d.key) - 1 ? key_len : sizeof(d.key) - 1;
    memcpy(d.key, key, n); d.key[n] = 0;
    memcpy(d.peaks, peaks, d.peaks_len);
    portENTER_CRITICAL(&lock);
    shared.detail = d;
    shared.detail_version++;
    shared.version++;
    portEXIT_CRITICAL(&lock);
}

void ruzina_load_display_note(const char *note)
{
    if (!ready) return;
    char copy[TEXT_BYTES];
    snprintf(copy, sizeof(copy), "%s", note);
    portENTER_CRITICAL(&lock);
    memcpy(shared.note, copy, sizeof(copy));
    shared.version++;
    portEXIT_CRITICAL(&lock);
}
