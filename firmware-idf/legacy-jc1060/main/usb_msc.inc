/* USB mass storage (Bulk-Only Transport, SCSI) for the media on a stick behind a hub on
 * the HS port.
 * Included by usb_host_enum.c, which owns the USB host client.
 *
 * Two tasks share this module:
 * - the USB client task binds and unbinds the stick (msc_bind, msc_unbind) and runs the
 *   transfer callbacks while it handles client events;
 * - the media owner task (sd_bridge.c) issues every SCSI command, and blocks on each
 *   transfer's completion. No SCSI command ever runs on the client task: its callbacks
 *   are what wakes the owner.
 *
 * Recovery follows the Bulk-Only Transport spec (USB MSC BOT 1.0, 5.3 and 6.6):
 * - a stalled data or status stage: clear the halt, then read the status (once more);
 * - an invalid or missing status, a phase error, or a timeout: Reset Recovery (the
 *   class Bulk-Only Mass Storage Reset, then clear both bulk halts);
 * - a failed READ (status 1): REQUEST SENSE, then the read is tried again.
 * A read is tried 3 times before the owner stops. It is tried again only after a Reset
 * Recovery whose every step succeeded: a recovery that fails gives the stick up, as a
 * transfer stuck in flight does.
 *
 * The two semaphores are static and made before the client task starts (msc_init), so
 * neither task ever sees one missing.
 *
 * Handle lifetime: the device handle is closed only while no command runs. The owner marks
 * each command busy before it checks that the stick is present, and the client marks the
 * stick gone before it checks busy (both sequentially consistent), so either the command
 * sees the stick gone, or the client waits for it to end before closing the device.
 *
 * A transfer is never submitted again while it may still be in flight. EP0 has no cancel
 * in this host library, so a control request that times out, or a bulk transfer still
 * running after its cancel, leaves the stick given up (`msc_stuck`) until it is unplugged
 * and the app restarts. Each completion wakes only the wait for its own transfer. */
#include <string.h>
#include "esp_system.h"
#include "freertos/semphr.h"

#define MSC_CLASS 0x08
#define MSC_SUBCLASS_SCSI 0x06
#define MSC_PROTOCOL_BOT 0x50
#define MSC_CBW_SIGNATURE 0x43425355u /* "USBC" */
#define MSC_CSW_SIGNATURE 0x53425355u /* "USBS" */
#define MSC_CBW_BYTES 31
#define MSC_CSW_BYTES 13
/* The largest data stage: the media bridge reads at most this much at a time. */
#define MSC_MAX_DATA_BYTES 8192
/* One bulk transfer's data on a full-speed stick: 4 KiB keeps it under the 127-packet
 * limit of an OTG 1.1 transfer (8128 bytes at 64-byte packets), so a larger data stage
 * goes in pieces. A high-speed stick (512-byte packets) takes the whole data stage in
 * one transfer: each transfer costs a round trip through the client task. */
#define MSC_CHUNK_BYTES 4096
#define MSC_HS_PACKET 512
static uint32_t msc_chunk_bytes(void);
#define MSC_TIMEOUT_MS 2000
#define MSC_READ_ATTEMPTS 3

/* What a command or transfer ended in, beyond esp_err_t's success. */
typedef enum {
    MSC_OK,
    MSC_STALL,     /* the endpoint stalled: clear it */
    MSC_FAILED,    /* status 1: the command failed; sense data says why */
    MSC_RECOVER,   /* invalid status, phase error, timeout: Reset Recovery */
    MSC_GONE,      /* the stick is gone */
} msc_result_t;

static usb_device_handle_t msc_dev;
static uint8_t msc_interface = 0xff, msc_in, msc_out;
static uint16_t msc_mps_in;
static usb_transfer_t *msc_transfer, *msc_control_transfer;
static StaticSemaphore_t msc_done_storage, msc_bound_storage;
static SemaphoreHandle_t msc_done, msc_bound;
/* Set once both semaphores exist; the owner waits for it before it takes msc_bound. */
static _Atomic bool msc_ready;
/* Set by the client task: the stick is bound, and not gone. */
static _Atomic bool msc_present;
/* Set by the owner task while a command uses the device handle. */
static _Atomic bool msc_busy;
/* A transfer may still be in flight after a timeout, or Reset Recovery failed: the stick
 * is given up. */
static bool msc_stuck;
/* The transfer the owner waits for; another one's completion does not wake it. */
static usb_transfer_t *_Atomic msc_awaited;
/* A stick went away in this run: the media owner has stopped for good. */
static bool msc_lost;
static uint32_t msc_tag;
static uint32_t msc_commands, msc_errors, msc_stalls, msc_resets, msc_retries;

/* The most one bulk transfer carries for the bound stick. */
static uint32_t msc_chunk_bytes(void)
{
    return msc_mps_in >= MSC_HS_PACKET ? MSC_MAX_DATA_BYTES : MSC_CHUNK_BYTES;
}

/* Before the USB client task starts: the semaphores both tasks use. Static storage, so
 * they cannot fail for want of memory. */
static void msc_init(void)
{
    msc_done = xSemaphoreCreateBinaryStatic(&msc_done_storage);
    msc_bound = xSemaphoreCreateBinaryStatic(&msc_bound_storage);
    msc_ready = true;
}

/* Owner task: whether the stick is still attached. */
bool ruzina_msc_present(void)
{
    return msc_present;
}

static void msc_transfer_done(usb_transfer_t *transfer)
{
    if (transfer == msc_awaited) xSemaphoreGive(msc_done);
}

/* Client task, on DEV_GONE: no new command may use the device from now on. */
static void msc_gone(void)
{
    msc_present = false;
    msc_lost = true;
}

/* Client task: whether the device may be closed, with no command using it. */
static bool msc_idle(void)
{
    return !msc_busy;
}

/* Whether `config` has a SCSI Bulk-Only interface; its number and bulk endpoints if so. */
static bool msc_find(const usb_config_desc_t *config, uint8_t *interface, uint8_t *in, uint8_t *out,
                     uint16_t *mps_in)
{
    const uint8_t *p = (const uint8_t *)config, *end = p + config->wTotalLength;
    bool inside = false;
    *in = *out = 0;
    for (p += config->bLength; p + 2 <= end && p[0] >= 2 && p + p[0] <= end; p += p[0]) {
        if (p[1] == 0x04 && p[0] >= 9) {
            if (inside && *in && *out) return true;
            inside = p[5] == MSC_CLASS && p[6] == MSC_SUBCLASS_SCSI && p[7] == MSC_PROTOCOL_BOT && p[3] == 0;
            if (inside) *interface = p[2];
            *in = *out = 0;
        } else if (inside && p[1] == 0x05 && p[0] >= 7 && (p[3] & 0x03) == 0x02) {
            if (p[2] & 0x80) {
                *in = p[2];
                *mps_in = p[4] | p[5] << 8;
            } else {
                *out = p[2];
            }
        }
    }
    return inside && *in && *out;
}

/* Client task: takes the device as the stick when it has a Bulk-Only interface. */
static bool msc_bind(usb_device_handle_t dev, const usb_config_desc_t *config)
{
    uint8_t interface, in, out;
    uint16_t mps;
    if (!msc_find(config, &interface, &in, &out, &mps)) return false;
    /* The media owner stopped when the last stick went, and the catalog and decks belong
     * to that stick: a stick put back restarts the app, which reads it from the start. A
     * restart is no harsher than a power cut, which the settings and Hot Cue stores take. */
    if (msc_lost) {
        printf("usb_msc reinserted restart=YES\n");
        fflush(stdout);
        vTaskDelay(pdMS_TO_TICKS(100));
        esp_restart();
    }
    if (msc_dev || !msc_ready) return false;
    if (!msc_transfer && usb_host_transfer_alloc(MSC_MAX_DATA_BYTES, 0, &msc_transfer) != ESP_OK) return false;
    if (!msc_control_transfer &&
        usb_host_transfer_alloc(sizeof(usb_setup_packet_t), 0, &msc_control_transfer) != ESP_OK)
        return false;
    esp_err_t err = usb_host_interface_claim(client, dev, interface, 0);
    if (err != ESP_OK) {
        printf("usb_msc claim=%s\n", esp_err_to_name(err));
        return false;
    }
    msc_dev = dev;
    msc_interface = interface;
    msc_in = in;
    msc_out = out;
    msc_mps_in = mps;
    msc_present = true;
    printf("usb_msc bound interface=%u in=%02x out=%02x mps=%u\n", interface, in, out, mps);
    fflush(stdout);
    xSemaphoreGive(msc_bound);
    return true;
}

/* Client task: the stick is gone, and no command uses it (msc_idle). */
static void msc_unbind(void)
{
    msc_present = false;
    msc_lost = true;
    usb_host_interface_release(client, msc_dev, msc_interface);
    usb_host_device_close(client, msc_dev);
    msc_dev = NULL;
    msc_interface = 0xff;
    printf("usb_msc unbound commands=%" PRIu32 " errors=%" PRIu32 " stalls=%" PRIu32 " resets=%" PRIu32
           " retries=%" PRIu32 "\n",
           msc_commands, msc_errors, msc_stalls, msc_resets, msc_retries);
    fflush(stdout);
}

/* Submits `t` and waits for it. On a timeout a bulk transfer is cancelled (halt, then
 * flush: the callback runs with CANCELED) and its endpoint left halted for recovery. A
 * transfer that has not ended by then (always so on EP0, which cannot be cancelled here)
 * gives the stick up, so it is never submitted again while in flight. */
static msc_result_t msc_submit_wait(usb_transfer_t *t)
{
    const uint8_t endpoint = t->bEndpointAddress;
    xSemaphoreTake(msc_done, 0);
    msc_awaited = t;
    esp_err_t err = endpoint ? usb_host_transfer_submit(t) : usb_host_transfer_submit_control(client, t);
    if (err != ESP_OK) {
        msc_awaited = NULL;
        return msc_present ? MSC_RECOVER : MSC_GONE;
    }
    bool ended = xSemaphoreTake(msc_done, pdMS_TO_TICKS(MSC_TIMEOUT_MS)) == pdTRUE;
    if (!ended && msc_present && endpoint) {
        usb_host_endpoint_halt(msc_dev, endpoint);
        usb_host_endpoint_flush(msc_dev, endpoint);
        ended = xSemaphoreTake(msc_done, pdMS_TO_TICKS(500)) == pdTRUE;
        if (ended) {
            msc_awaited = NULL;
            return MSC_RECOVER;
        }
    }
    /* A gone device's transfers end with NO_DEVICE: wait for that before letting go. */
    if (!ended && !msc_present) ended = xSemaphoreTake(msc_done, pdMS_TO_TICKS(MSC_TIMEOUT_MS)) == pdTRUE;
    msc_awaited = NULL;
    if (!ended) {
        msc_stuck = true;
        printf("usb_msc stuck endpoint=%02x present=%d reinsert=APP_RESTART_REQUIRED\n", endpoint,
               (int)msc_present);
        fflush(stdout);
        return MSC_GONE;
    }
    if (!msc_present) return MSC_GONE;
    switch (t->status) {
        case USB_TRANSFER_STATUS_COMPLETED: return MSC_OK;
        case USB_TRANSFER_STATUS_STALL: msc_stalls++; return MSC_STALL;
        case USB_TRANSFER_STATUS_NO_DEVICE: return MSC_GONE;
        default: return MSC_RECOVER;
    }
}

/* A control request with no data stage, waited on. */
static msc_result_t msc_control(uint8_t request_type, uint8_t request, uint16_t value, uint16_t index)
{
    if (!msc_present || msc_stuck) return MSC_GONE;
    usb_transfer_t *t = msc_control_transfer;
    usb_setup_packet_t *setup = (usb_setup_packet_t *)t->data_buffer;
    setup->bmRequestType = request_type;
    setup->bRequest = request;
    setup->wValue = value;
    setup->wIndex = index;
    setup->wLength = 0;
    t->device_handle = msc_dev;
    t->bEndpointAddress = 0;
    t->num_bytes = sizeof(usb_setup_packet_t);
    t->callback = msc_transfer_done;
    t->context = NULL;
    t->timeout_ms = MSC_TIMEOUT_MS;
    return msc_submit_wait(t);
}

/* Clears a halted bulk endpoint on both sides: the host pipe's and the device's
 * (CLEAR_FEATURE ENDPOINT_HALT). MSC_OK, MSC_RECOVER when it did not clear, or MSC_GONE. */
static msc_result_t msc_clear_halt(uint8_t endpoint)
{
    if (!msc_present || msc_stuck) return MSC_GONE;
    /* Halting a pipe the stall already halted is allowed and returns ESP_OK. Flush and clear
     * need a halted pipe, so after the halt any error is a real fault. */
    esp_err_t err = usb_host_endpoint_halt(msc_dev, endpoint);
    if (err == ESP_OK) err = usb_host_endpoint_flush(msc_dev, endpoint);
    if (err == ESP_OK) err = usb_host_endpoint_clear(msc_dev, endpoint);
    if (err != ESP_OK) {
        printf("usb_msc clear_halt endpoint=%02x host=%s\n", endpoint, esp_err_to_name(err));
        return msc_present ? MSC_RECOVER : MSC_GONE;
    }
    const msc_result_t r = msc_control(0x02, 0x01, 0, endpoint); /* standard, endpoint: CLEAR_FEATURE(ENDPOINT_HALT) */
    return r == MSC_STALL ? MSC_RECOVER : r;
}

/* Bulk-Only Reset Recovery: the class reset, then both bulk endpoints cleared. Every step
 * must succeed: the outcome is the stick going away if it did, else the first step that
 * failed, else MSC_OK. */
static msc_result_t msc_reset_recovery(void)
{
    msc_resets++;
    msc_result_t reset = msc_control(0x21, 0xff, 0, msc_interface); /* class, interface: Bulk-Only Mass Storage Reset */
    if (reset == MSC_STALL) reset = MSC_RECOVER;
    const msc_result_t in = msc_clear_halt(msc_in);
    const msc_result_t out = msc_clear_halt(msc_out);
    msc_result_t r = reset != MSC_OK ? reset : in != MSC_OK ? in : out;
    if (reset == MSC_GONE || in == MSC_GONE || out == MSC_GONE) r = MSC_GONE;
    printf("usb_msc reset_recovery n=%" PRIu32 " reset=%d in=%d out=%d result=%d\n", msc_resets, (int)reset,
           (int)in, (int)out, (int)r);
    fflush(stdout);
    return r;
}

/* One bulk transfer of `len` bytes (at most one chunk). `buffer` is copied in for OUT and
 * out for IN; `actual` gets the bytes moved. */
static msc_result_t msc_bulk(uint8_t endpoint, uint8_t *buffer, size_t len, size_t *actual)
{
    if (!msc_present || msc_stuck) return MSC_GONE;
    usb_transfer_t *t = msc_transfer;
    const bool in = endpoint & 0x80;
    /* An IN transfer must ask for whole packets. */
    size_t request = in ? (len + msc_mps_in - 1) / msc_mps_in * msc_mps_in : len;
    if (request > t->data_buffer_size || request > msc_chunk_bytes()) return MSC_RECOVER;
    if (!in) memcpy(t->data_buffer, buffer, len);
    t->device_handle = msc_dev;
    t->bEndpointAddress = endpoint;
    t->num_bytes = (int)request;
    t->callback = msc_transfer_done;
    t->context = NULL;
    t->timeout_ms = MSC_TIMEOUT_MS;
    msc_result_t r = msc_submit_wait(t);
    if (r != MSC_OK) return r;
    size_t got = (size_t)t->actual_num_bytes;
    if (in) memcpy(buffer, t->data_buffer, got < len ? got : len);
    if (actual) *actual = got;
    return MSC_OK;
}

/* Reads the status stage; a stall there is cleared and the status read once more. */
static msc_result_t msc_status(uint32_t tag, uint8_t *status)
{
    uint8_t csw[MSC_CSW_BYTES] = {0};
    size_t got = 0;
    msc_result_t r = msc_bulk(msc_in, csw, sizeof csw, &got);
    if (r == MSC_STALL) {
        r = msc_clear_halt(msc_in);
        if (r == MSC_OK) r = msc_bulk(msc_in, csw, sizeof csw, &got);
    }
    if (r != MSC_OK) return r == MSC_STALL ? MSC_RECOVER : r;
    uint32_t signature, csw_tag;
    memcpy(&signature, csw, 4);
    memcpy(&csw_tag, csw + 4, 4);
    if (got != MSC_CSW_BYTES || signature != MSC_CSW_SIGNATURE || csw_tag != tag || csw[12] > 1)
        return MSC_RECOVER; /* not a valid status, or a phase error (2) */
    *status = csw[12];
    return MSC_OK;
}

/* One SCSI command: CBW, an optional IN data stage of `len` bytes in chunks, CSW. A
 * command that needs Reset Recovery gets it before returning. Runs while busy. */
static msc_result_t msc_command_run(const uint8_t *cdb, uint8_t cdb_len, uint8_t *data, uint32_t len)
{
    uint8_t cbw[MSC_CBW_BYTES] = {0};
    const uint32_t tag = ++msc_tag;
    const uint32_t signature = MSC_CBW_SIGNATURE;
    msc_commands++;
    memcpy(cbw, &signature, 4);
    memcpy(cbw + 4, &tag, 4);
    memcpy(cbw + 8, &len, 4);
    cbw[12] = len ? 0x80 : 0x00; /* data in */
    cbw[13] = 0;                 /* LUN 0 */
    cbw[14] = cdb_len;
    memcpy(cbw + 15, cdb, cdb_len);
    msc_result_t r = msc_bulk(msc_out, cbw, sizeof cbw, NULL);
    uint32_t done = 0;
    if (r == MSC_OK) {
        /* The data stage in pieces; a short piece ends it early. */
        while (done < len) {
            const uint32_t chunk = msc_chunk_bytes();
            const uint32_t want = len - done < chunk ? len - done : chunk;
            size_t got = 0;
            r = msc_bulk(msc_in, data + done, want, &got);
            if (r != MSC_OK) break;
            done += got < want ? (uint32_t)got : want;
            if (got < want) break;
        }
        /* A stalled data stage still has its status to read (BOT 6.7.2). */
        if (r == MSC_STALL) r = msc_clear_halt(msc_in);
    } else if (r == MSC_STALL) {
        r = MSC_RECOVER; /* a stalled CBW: the device needs a reset */
    }
    uint8_t status = 0;
    if (r == MSC_OK) r = msc_status(tag, &status);
    if (r == MSC_OK && status == 1) r = MSC_FAILED;
    if (r == MSC_OK && done != len) r = MSC_FAILED;
    if (r != MSC_OK) msc_errors++;
    if (r == MSC_RECOVER) {
        /* Tried again only on a stick that recovered; one that did not is given up. */
        const msc_result_t recovered = msc_reset_recovery();
        if (recovered == MSC_GONE) {
            r = MSC_GONE;
        } else if (recovered != MSC_OK) {
            msc_stuck = true;
            printf("usb_msc recovery_failed reinsert=APP_RESTART_REQUIRED\n");
            fflush(stdout);
            r = MSC_GONE;
        }
    }
    return r;
}

/* Owner task: one command, with the device handle kept open until it ends. Busy is set
 * before presence is read; the client marks the stick gone before it reads busy. */
static msc_result_t msc_command(const uint8_t *cdb, uint8_t cdb_len, uint8_t *data, uint32_t len)
{
    msc_busy = true;
    const msc_result_t r = msc_present && !msc_stuck ? msc_command_run(cdb, cdb_len, data, len) : MSC_GONE;
    msc_busy = false;
    return r;
}

static esp_err_t msc_err(msc_result_t r)
{
    switch (r) {
        case MSC_OK: return ESP_OK;
        case MSC_GONE: return ESP_ERR_NOT_FOUND;
        case MSC_FAILED: return ESP_ERR_INVALID_STATE;
        default: return ESP_FAIL;
    }
}

/* Asks the stick why the last command failed, and logs it. */
static void msc_sense(const char *what)
{
    uint8_t sense[18] = {0};
    const uint8_t request_sense[6] = {0x03, 0, 0, 0, sizeof sense, 0};
    if (msc_command(request_sense, sizeof request_sense, sense, sizeof sense) == MSC_OK)
        printf("usb_msc sense after=%s key=%x asc=%02x ascq=%02x\n", what, sense[2] & 0x0f, sense[12], sense[13]);
}

/* Owner task: waits for a stick, makes it ready and returns its 512-byte sector count. */
esp_err_t ruzina_msc_open(uint32_t *sectors)
{
    while (!msc_ready) vTaskDelay(pdMS_TO_TICKS(100));
    xSemaphoreTake(msc_bound, portMAX_DELAY);
    uint8_t buffer[36];
    const uint8_t inquiry[6] = {0x12, 0, 0, 0, sizeof buffer, 0};
    msc_result_t r = msc_command(inquiry, sizeof inquiry, buffer, sizeof buffer);
    if (r != MSC_OK) return msc_err(r);
    printf("usb_msc inquiry type=%02x removable=%u vendor=\"%.8s\" product=\"%.16s\"\n", buffer[0] & 0x1f,
           buffer[1] >> 7, (const char *)buffer + 8, (const char *)buffer + 16);
    /* A stick may need a moment after attach: TEST UNIT READY until it reports ready. */
    const uint8_t ready[6] = {0};
    for (int attempt = 0;; attempt++) {
        r = msc_command(ready, sizeof ready, NULL, 0);
        if (r == MSC_OK) break;
        if (r == MSC_GONE || attempt >= 20) return msc_err(r);
        if (r == MSC_FAILED) msc_sense("test_unit_ready");
        vTaskDelay(pdMS_TO_TICKS(100));
    }
    uint8_t capacity[8];
    const uint8_t read_capacity[10] = {0x25};
    r = msc_command(read_capacity, sizeof read_capacity, capacity, sizeof capacity);
    if (r != MSC_OK) return msc_err(r);
    const uint32_t last = (uint32_t)capacity[0] << 24 | capacity[1] << 16 | capacity[2] << 8 | capacity[3];
    const uint32_t block = (uint32_t)capacity[4] << 24 | capacity[5] << 16 | capacity[6] << 8 | capacity[7];
    printf("usb_msc capacity last_lba=%" PRIu32 " block_bytes=%" PRIu32 "\n", last, block);
    fflush(stdout);
    /* The media stack reads 512-byte sectors; READ CAPACITY(16) sticks (2 TiB+) are out. */
    if (block != 512 || last == 0xffffffffu) return ESP_ERR_NOT_SUPPORTED;
    *sectors = last + 1;
    return ESP_OK;
}

/* Owner task: reads `count` 512-byte sectors from `first` into `data`, trying again after
 * a failed or recovered attempt. */
esp_err_t ruzina_msc_read(uint32_t first, uint32_t count, uint8_t *data)
{
    const uint32_t len = count * 512u;
    if (count == 0 || len > MSC_MAX_DATA_BYTES || count > 0xffff) return ESP_ERR_INVALID_ARG;
    const uint8_t read10[10] = {0x28, 0, (uint8_t)(first >> 24), (uint8_t)(first >> 16), (uint8_t)(first >> 8),
                                (uint8_t)first, 0, (uint8_t)(count >> 8), (uint8_t)count, 0};
    msc_result_t r = MSC_RECOVER;
    for (int attempt = 0; attempt < MSC_READ_ATTEMPTS; attempt++) {
        if (attempt) msc_retries++;
        r = msc_command(read10, sizeof read10, data, len);
        if (r == MSC_OK || r == MSC_GONE) break;
        if (r == MSC_FAILED) msc_sense("read");
        printf("usb_msc read_retry first=%" PRIu32 " count=%" PRIu32 " attempt=%d result=%d\n", first, count,
               attempt + 1, (int)r);
        fflush(stdout);
    }
    return msc_err(r);
}
