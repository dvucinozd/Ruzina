/* Compile the stick's actual driver (main/usb_msc.inc) against a fake Bulk-Only stick and
 * inject its faults: the bind's failure points, and each step of Reset Recovery. */
#include <assert.h>
#include <inttypes.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef int esp_err_t;
#define ESP_OK 0
#define ESP_FAIL (-1)
#define ESP_ERR_NO_MEM 0x101
#define ESP_ERR_INVALID_ARG 0x102
#define ESP_ERR_INVALID_STATE 0x103
#define ESP_ERR_NOT_FOUND 0x105
#define ESP_ERR_NOT_SUPPORTED 0x106
static const char *esp_err_to_name(esp_err_t err) { return err == ESP_OK ? "ESP_OK" : "ESP_FAIL"; }
static void esp_restart(void) { assert(!"no restart in these tests"); }

typedef unsigned TickType_t;
#define pdMS_TO_TICKS(ms) ((TickType_t)(ms))
#define portMAX_DELAY 0xffffffffu
#define pdTRUE 1
#define pdFALSE 0
static void vTaskDelay(TickType_t ticks) { (void)ticks; }
/* A binary semaphore that never blocks: the fake stick completes every transfer at once. */
typedef struct { int count; } StaticSemaphore_t;
typedef StaticSemaphore_t *SemaphoreHandle_t;
static SemaphoreHandle_t xSemaphoreCreateBinaryStatic(StaticSemaphore_t *s) { s->count = 0; return s; }
static int xSemaphoreTake(SemaphoreHandle_t s, TickType_t ticks)
{
    (void)ticks;
    assert(s);
    if (!s->count) return pdFALSE;
    s->count = 0;
    return pdTRUE;
}
static int xSemaphoreGive(SemaphoreHandle_t s)
{
    assert(s);
    s->count = 1;
    return pdTRUE;
}

typedef void *usb_device_handle_t;
typedef void *usb_host_client_handle_t;
enum {
    USB_TRANSFER_STATUS_COMPLETED,
    USB_TRANSFER_STATUS_ERROR,
    USB_TRANSFER_STATUS_TIMED_OUT,
    USB_TRANSFER_STATUS_CANCELED,
    USB_TRANSFER_STATUS_STALL,
    USB_TRANSFER_STATUS_OVERFLOW,
    USB_TRANSFER_STATUS_SKIPPED,
    USB_TRANSFER_STATUS_NO_DEVICE,
};
typedef struct usb_transfer {
    uint8_t *data_buffer;
    size_t data_buffer_size;
    int num_bytes;
    int actual_num_bytes;
    usb_device_handle_t device_handle;
    uint8_t bEndpointAddress;
    int status;
    uint32_t timeout_ms;
    void (*callback)(struct usb_transfer *);
    void *context;
} usb_transfer_t;
typedef struct __attribute__((packed)) {
    uint8_t bmRequestType, bRequest;
    uint16_t wValue, wIndex, wLength;
} usb_setup_packet_t;
typedef struct __attribute__((packed)) {
    uint8_t bLength, bDescriptorType;
    uint16_t wTotalLength;
    uint8_t bNumInterfaces, bConfigurationValue, iConfiguration, bmAttributes, bMaxPower;
} usb_config_desc_t;
static usb_host_client_handle_t client = (void *)1;

/* The fake stick: Bulk-Only Transport on IN 0x81 and OUT 0x02, with its faults. */
enum stage { IDLE, DATA, STATUS };
static struct {
    enum stage stage;
    uint32_t tag, left;
    uint8_t opcode;
    int bad_csw;              /* statuses still to send invalid */
    bool stall_reset;         /* the class reset stalls */
    bool stall_clear_out;     /* CLEAR_FEATURE on the OUT endpoint stalls */
    esp_err_t host_clear_in;  /* the host's clear of the IN pipe returns this */
    int reads, resets, clears;
} stick;
static int fake_device;
static int alloc_calls, alloc_fail_at = -1;
static bool claim_fails;

static esp_err_t usb_host_transfer_alloc(size_t size, int isoc, usb_transfer_t **out)
{
    (void)isoc;
    if (alloc_calls++ == alloc_fail_at) return ESP_ERR_NO_MEM;
    usb_transfer_t *t = calloc(1, sizeof *t);
    t->data_buffer = calloc(1, size);
    t->data_buffer_size = size;
    *out = t;
    return ESP_OK;
}
static esp_err_t usb_host_interface_claim(usb_host_client_handle_t c, usb_device_handle_t d, uint8_t i, uint8_t alt)
{
    (void)c, (void)d, (void)i, (void)alt;
    return claim_fails ? ESP_FAIL : ESP_OK;
}
static esp_err_t usb_host_interface_release(usb_host_client_handle_t c, usb_device_handle_t d, uint8_t i)
{
    (void)c, (void)d, (void)i;
    return ESP_OK;
}
static esp_err_t usb_host_device_close(usb_host_client_handle_t c, usb_device_handle_t d)
{
    (void)c, (void)d;
    return ESP_OK;
}
static esp_err_t usb_host_endpoint_halt(usb_device_handle_t d, uint8_t endpoint)
{
    (void)d, (void)endpoint;
    return ESP_OK;
}
static esp_err_t usb_host_endpoint_flush(usb_device_handle_t d, uint8_t endpoint)
{
    (void)d, (void)endpoint;
    return ESP_OK;
}
static esp_err_t usb_host_endpoint_clear(usb_device_handle_t d, uint8_t endpoint)
{
    (void)d;
    return endpoint == 0x81 ? stick.host_clear_in : ESP_OK;
}

static void complete(usb_transfer_t *t, int status, int actual)
{
    t->status = status;
    t->actual_num_bytes = actual;
    t->callback(t);
}

static esp_err_t usb_host_transfer_submit(usb_transfer_t *t)
{
    if (t->bEndpointAddress == 0x02) {
        uint32_t signature;
        assert(t->num_bytes == 31);
        memcpy(&signature, t->data_buffer, 4);
        assert(signature == 0x43425355u);
        memcpy(&stick.tag, t->data_buffer + 4, 4);
        memcpy(&stick.left, t->data_buffer + 8, 4);
        stick.opcode = t->data_buffer[15];
        if (stick.opcode == 0x28) stick.reads++;
        stick.stage = stick.left ? DATA : STATUS;
        complete(t, USB_TRANSFER_STATUS_COMPLETED, t->num_bytes);
        return ESP_OK;
    }
    assert(t->bEndpointAddress == 0x81);
    if (stick.stage == DATA) {
        const uint32_t n = stick.left < (uint32_t)t->num_bytes ? stick.left : (uint32_t)t->num_bytes;
        static const uint8_t capacity[8] = {0, 0, 0x03, 0xe8, 0, 0, 0x02, 0}; /* last LBA 1000, 512 B */
        if (stick.opcode == 0x25) memcpy(t->data_buffer, capacity, sizeof capacity);
        else memset(t->data_buffer, 0x5a, n);
        stick.left -= n;
        if (!stick.left) stick.stage = STATUS;
        complete(t, USB_TRANSFER_STATUS_COMPLETED, (int)n);
        return ESP_OK;
    }
    assert(stick.stage == STATUS);
    uint8_t csw[13] = {0};
    const uint32_t signature = stick.bad_csw ? 0 : 0x53425355u;
    if (stick.bad_csw) stick.bad_csw--;
    memcpy(csw, &signature, 4);
    memcpy(csw + 4, &stick.tag, 4);
    memcpy(t->data_buffer, csw, sizeof csw);
    stick.stage = IDLE;
    complete(t, USB_TRANSFER_STATUS_COMPLETED, (int)sizeof csw);
    return ESP_OK;
}

static esp_err_t usb_host_transfer_submit_control(usb_host_client_handle_t c, usb_transfer_t *t)
{
    (void)c;
    const usb_setup_packet_t *setup = (const usb_setup_packet_t *)t->data_buffer;
    int status = USB_TRANSFER_STATUS_COMPLETED;
    if (setup->bRequest == 0xff) {
        stick.resets++;
        stick.stage = IDLE;
        if (stick.stall_reset) status = USB_TRANSFER_STATUS_STALL;
    } else if (setup->bRequest == 0x01) {
        stick.clears++;
        if (setup->wIndex == 0x02 && stick.stall_clear_out) status = USB_TRANSFER_STATUS_STALL;
    }
    complete(t, status, 0);
    return ESP_OK;
}

#include "usb_msc.inc"

int main(void)
{
    /* Configuration, a SCSI Bulk-Only interface, bulk IN 0x81 and OUT 0x02 at 512 bytes. */
    static const uint8_t config[] = {9, 2, 32, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 2, 8, 6, 0x50, 0,
                                     7, 5, 0x81, 2, 0x00, 0x02, 0, 7, 5, 0x02, 2, 0x00, 0x02, 0};
    const usb_config_desc_t *desc = (const usb_config_desc_t *)config;
    usb_device_handle_t dev = &fake_device;
    uint8_t data[1024];
    uint32_t sectors = 0;
    (void)msc_gone, (void)msc_idle, (void)msc_unbind, (void)ruzina_msc_present;

    /* Finding 2: nothing binds before the semaphores exist, every failure point leaves the
     * stick unbound, and the next attach binds. */
    assert(!msc_bind(dev, desc));
    msc_init();
    assert(msc_done && msc_bound && msc_ready);
    alloc_fail_at = 0; /* the data transfer */
    assert(!msc_bind(dev, desc));
    assert(!msc_present && !msc_dev && !msc_transfer);
    alloc_fail_at = 2; /* the data transfer is made this time; the control one fails */
    assert(!msc_bind(dev, desc));
    assert(!msc_present && !msc_dev && msc_transfer && !msc_control_transfer);
    alloc_fail_at = -1;
    claim_fails = true;
    assert(!msc_bind(dev, desc));
    assert(!msc_present && !msc_dev);
    claim_fails = false;
    assert(msc_bind(dev, desc));
    assert(msc_present && msc_dev == dev && msc_bound->count == 1);
    assert(ruzina_msc_open(&sectors) == ESP_OK && sectors == 1001);
    assert(ruzina_msc_read(0, 2, data) == ESP_OK && stick.reads == 1 && data[1023] == 0x5a);

    /* Finding 3: a Reset Recovery whose every step succeeds lets the read try again. */
    stick.reads = 0;
    stick.bad_csw = 1;
    assert(ruzina_msc_read(0, 2, data) == ESP_OK);
    assert(stick.reads == 2 && msc_resets == 1 && !msc_stuck);

    /* One failed step gives the stick up: the read stops after its first attempt, and
     * nothing more goes to the stick. Before, a later step's success hid the failure. */
    const struct {
        const char *what;
        bool stall_reset, stall_clear_out;
        esp_err_t host_clear_in;
    } cases[] = {
        {"class reset stalls", true, false, ESP_OK},
        {"IN pipe does not clear", false, false, ESP_FAIL},
        {"OUT CLEAR_FEATURE stalls", false, true, ESP_OK},
    };
    for (size_t i = 0; i < sizeof cases / sizeof cases[0]; i++) {
        stick.reads = 0;
        stick.bad_csw = 1;
        stick.stall_reset = cases[i].stall_reset;
        stick.stall_clear_out = cases[i].stall_clear_out;
        stick.host_clear_in = cases[i].host_clear_in;
        const int resets = stick.resets;
        assert(ruzina_msc_read(0, 2, data) == ESP_ERR_NOT_FOUND);
        assert(stick.reads == 1 && stick.resets == resets + 1 && msc_stuck);
        assert(ruzina_msc_read(0, 2, data) == ESP_ERR_NOT_FOUND);
        assert(stick.reads == 1 && stick.resets == resets + 1);
        printf("usb_msc_test case=\"%s\" given_up=1\n", cases[i].what);
        msc_stuck = false;
        stick.stall_reset = stick.stall_clear_out = false;
        stick.host_clear_in = ESP_OK;
    }
    puts("usb_msc_test ok");
    return 0;
}
