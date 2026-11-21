/* When the deck screen may start a step of memory work (main/fill_gap.h). Fills here come
 * every 6000 us and take 2500 us, as Master Tempo fills do on the board. */
#include <assert.h>
#include <stdio.h>
#include "fill_gap.h"

int main(void)
{
    const uint32_t base = 1000000;
    fill_gap_t f = {.start_us = base, .end_us = base + 2500, .period_us = 6000};

    /* Never during a fill. */
    f.running = true;
    assert(!fill_gap_open(f, base + 1000, 0));
    f.running = false;

    /* Just after a fill, a step starts whatever its length. */
    assert(fill_gap_open(f, base + 2500, 100000));
    assert(fill_gap_open(f, base + 2500 + FILL_GAP_FRESH_US, 100000));

    /* Later in the gap only a step that fits before the next fill at base + 6000. */
    assert(fill_gap_open(f, base + 4000, 2000));
    assert(!fill_gap_open(f, base + 4000, 2001));
    assert(!fill_gap_open(f, base + 5900, 200));

    /* A late fill is due any moment. */
    assert(!fill_gap_open(f, base + 6100, 0));

    /* Without a stream, or before the first period is known, nothing waits. */
    assert(fill_gap_open(f, base + FILL_GAP_IDLE_US, 100000));
    fill_gap_t first = {.start_us = base, .end_us = base + 2500};
    assert(fill_gap_open(first, base + 4000, 100000));

    /* The uptime's low 32 bits come round. */
    fill_gap_t wrap = {.start_us = UINT32_MAX - 999, .end_us = 1500, .period_us = 6000};
    assert(!fill_gap_open(wrap, 4000, 2001));
    assert(fill_gap_open(wrap, 4000, 1000));

    /* The period follows the fills, and ignores gaps no stream has. */
    assert(fill_gap_period(base, base + 6000, 0) == 6000);
    assert(fill_gap_period(base, base + 5900, 6000) == 5900);
    assert(fill_gap_period(base, base + 500, 6000) == 6000);
    assert(fill_gap_period(base, base + 50000, 6000) == 6000);
    assert(fill_gap_period(UINT32_MAX - 2999, 3000, 0) == 6000);
    puts("fill_gap_test ok");
    return 0;
}
