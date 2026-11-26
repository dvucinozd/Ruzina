#pragma once
#include <stdbool.h>
#include <stdint.h>

/* The HS controller's FIFO split, in 4-byte lines. esp-usb's balanced split gives periodic
 * OUT 128 lines (512 bytes), under the Inpulse 500's 540-byte isochronous packet, so the
 * deck's interface claim fails. This split keeps the balanced non-periodic share, moves 32
 * lines from RX to periodic OUT, and leaves RX all the rest (the donor's JC1060 split). */
#define USB_FIFO_NPTX_LINES 256u   /* 1 KiB: two 512-byte bulk OUT packets (stick, MIDI) */
#define USB_FIFO_PTX_LINES 160u    /* 640 bytes: one 540-byte isochronous OUT packet */
#define USB_FIFO_RX_MIN_LINES 258u /* two 512-byte bulk IN packets and two status lines */

typedef struct {
    uint32_t rx, nptx, ptx;
} usb_fifo_plan_t;

/* The split for a FIFO of `total_lines`; false, and esp-usb's own split, when it is too
 * small for one. */
static inline bool usb_fifo_plan(uint32_t total_lines, usb_fifo_plan_t *out)
{
    if (!out || total_lines < USB_FIFO_NPTX_LINES + USB_FIFO_PTX_LINES + USB_FIFO_RX_MIN_LINES)
        return false;
    *out = (usb_fifo_plan_t){.rx = total_lines - USB_FIFO_NPTX_LINES - USB_FIFO_PTX_LINES,
                             .nptx = USB_FIFO_NPTX_LINES, .ptx = USB_FIFO_PTX_LINES};
    return true;
}
