#include <assert.h>
#include <stddef.h>
#include "usb_fifo.h"

int main(void)
{
    usb_fifo_plan_t plan;
    /* A 4 KiB HS FIFO: non-periodic as esp-usb's balanced split, 32 lines from RX to
     * periodic OUT. Every line is used, and none twice. */
    assert(usb_fifo_plan(1024, &plan));
    assert(plan.nptx == 256 && plan.ptx == 160 && plan.rx == 608);
    assert(plan.rx + plan.nptx + plan.ptx == 1024);
    /* The Inpulse 500's 540-byte isochronous packet fits; esp-usb's 128 lines did not. */
    assert(plan.ptx * 4 >= 540 && 128 * 4 < 540);
    /* RX still holds two 512-byte bulk IN packets and two status lines. */
    assert(plan.rx * 4 >= 2 * 512 + 2 * 4);
    /* The smallest FIFO that takes the split, and one line less: esp-usb's own. */
    assert(usb_fifo_plan(674, &plan) && plan.rx == USB_FIFO_RX_MIN_LINES);
    plan = (usb_fifo_plan_t){1, 2, 3};
    assert(!usb_fifo_plan(673, &plan) && plan.rx == 1 && plan.nptx == 2 && plan.ptx == 3);
    assert(!usb_fifo_plan(256, &plan) && !usb_fifo_plan(0, &plan));
    assert(!usb_fifo_plan(1024, NULL));
    return 0;
}
