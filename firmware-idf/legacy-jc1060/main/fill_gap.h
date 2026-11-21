#pragma once
/* When the deck screen may start a step of memory work: in the gap between two USB audio
 * fills, and only if the step fits before the next fill. On s163 a screen frame during a
 * fill made 88 % of the Master Tempo fills over 3 ms, the two competing for PSRAM.
 *
 * Fills come one per transfer, about every 6 ms, and take 1.5 to 3 ms. Times are the
 * uptime's low 32 bits. Shared with a host test (tools/tests/fill_gap_test.c). */
#include <stdbool.h>
#include <stdint.h>

enum {
    /* Just after a fill ends, a step starts whatever its length: no later moment in the
     * gap gives it more time before the next fill. */
    FILL_GAP_FRESH_US = 300,
    /* Without a fill for this long there is no stream to keep out of the way. */
    FILL_GAP_IDLE_US = 20000,
    /* Periods outside these are not a stream's. */
    FILL_GAP_PERIOD_MIN_US = 1000,
    FILL_GAP_PERIOD_MAX_US = 12000,
};

typedef struct {
    bool running;
    uint32_t start_us, end_us, period_us;
} fill_gap_t;

/* The last fill's start, end and the period between fills, as the fill saw them. */
static inline uint32_t fill_gap_period(uint32_t previous_start_us, uint32_t start_us, uint32_t period_us)
{
    const uint32_t step = start_us - previous_start_us;
    return step >= FILL_GAP_PERIOD_MIN_US && step <= FILL_GAP_PERIOD_MAX_US ? step : period_us;
}

/* Whether a step expected to take `budget_us` may start at `now_us`. */
static inline bool fill_gap_open(fill_gap_t f, uint32_t now_us, uint32_t budget_us)
{
    if (f.running) return false;
    if (now_us - f.end_us <= FILL_GAP_FRESH_US) return true;
    if (!f.period_us || now_us - f.start_us >= FILL_GAP_IDLE_US) return true;
    const int32_t until_next = (int32_t)(f.start_us + f.period_us - now_us);
    /* A late fill is due any moment: wait for it. */
    if (until_next <= 0) return false;
    return (uint32_t)until_next >= budget_us;
}
