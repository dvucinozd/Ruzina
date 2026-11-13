/* USB mass storage (Bulk-Only Transport, SCSI) for the media on a stick on the FS port.
 * Included by usb_host_enum.c, which owns the USB host client.
 *
 * Two tasks share this module:
 * - the USB client task binds and unbinds the stick (msc_bind, msc_unbind) and runs the
 *   transfer callbacks while it handles client events;
 * - the media owner task (sd_bridge.c) issues every SCSI command, and blocks on each
 *   transfer's completion. No SCSI command ever runs on the client task: its callbacks
 *   are what wakes the owner.
 *
 * Errors are not recovered yet: a failed command makes the owner stop, as an SD error
 * does, and the stick needs an app restart. */
#include <string.h>
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
#define MSC_TIMEOUT_MS 2000

static usb_device_handle_t msc_dev;
static uint8_t msc_interface = 0xff, msc_in, msc_out;
static uint16_t msc_mps_in;
static usb_transfer_t *msc_transfer;
static SemaphoreHandle_t msc_done, msc_bound;
static volatile bool msc_present;
static uint32_t msc_tag;
static uint32_t msc_commands, msc_errors;

/* Owner task: whether the stick is still attached. */
bool ruzina_msc_present(void)
{
    return msc_present;
}

static void msc_transfer_done(usb_transfer_t *transfer)
{
    (void)transfer;
    xSemaphoreGive(msc_done);
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
    if (msc_dev || !msc_find(config, &interface, &in, &out, &mps)) return false;
    if (!msc_done) msc_done = xSemaphoreCreateBinary();
    if (!msc_bound) msc_bound = xSemaphoreCreateBinary();
    if (!msc_transfer && usb_host_transfer_alloc(MSC_MAX_DATA_BYTES, 0, &msc_transfer) != ESP_OK) return false;
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

/* Client task: the stick is gone. In-flight transfers have completed by now. */
static void msc_unbind(void)
{
    msc_present = false;
    usb_host_interface_release(client, msc_dev, msc_interface);
    usb_host_device_close(client, msc_dev);
    msc_dev = NULL;
    msc_interface = 0xff;
    printf("usb_msc unbound commands=%" PRIu32 " errors=%" PRIu32 "\n", msc_commands, msc_errors);
    fflush(stdout);
}

/* One bulk transfer of `len` bytes, waited on. `buffer` is copied in for OUT and out for
 * IN; `actual` gets the bytes moved. */
static esp_err_t msc_bulk(uint8_t endpoint, uint8_t *buffer, size_t len, size_t *actual)
{
    if (!msc_present) return ESP_ERR_INVALID_STATE;
    usb_transfer_t *t = msc_transfer;
    const bool in = endpoint & 0x80;
    /* An IN transfer must ask for whole packets. */
    size_t request = in ? (len + msc_mps_in - 1) / msc_mps_in * msc_mps_in : len;
    if (request > t->data_buffer_size) return ESP_ERR_INVALID_SIZE;
    if (!in) memcpy(t->data_buffer, buffer, len);
    t->device_handle = msc_dev;
    t->bEndpointAddress = endpoint;
    t->num_bytes = (int)request;
    t->callback = msc_transfer_done;
    t->context = NULL;
    t->timeout_ms = MSC_TIMEOUT_MS;
    xSemaphoreTake(msc_done, 0);
    esp_err_t err = usb_host_transfer_submit(t);
    if (err != ESP_OK) return err;
    if (xSemaphoreTake(msc_done, pdMS_TO_TICKS(MSC_TIMEOUT_MS + 500)) != pdTRUE) return ESP_ERR_TIMEOUT;
    if (t->status != USB_TRANSFER_STATUS_COMPLETED) return ESP_FAIL;
    size_t got = (size_t)t->actual_num_bytes;
    if (in) memcpy(buffer, t->data_buffer, got < len ? got : len);
    if (actual) *actual = got;
    return ESP_OK;
}

/* One SCSI command: CBW, an optional IN data stage of `len` bytes, CSW. */
static esp_err_t msc_command(const uint8_t *cdb, uint8_t cdb_len, uint8_t *data, uint32_t len)
{
    uint8_t cbw[MSC_CBW_BYTES] = {0};
    const uint32_t tag = ++msc_tag;
    msc_commands++;
    memcpy(cbw, &(uint32_t){MSC_CBW_SIGNATURE}, 4);
    memcpy(cbw + 4, &tag, 4);
    memcpy(cbw + 8, &len, 4);
    cbw[12] = len ? 0x80 : 0x00; /* data in */
    cbw[13] = 0;                 /* LUN 0 */
    cbw[14] = cdb_len;
    memcpy(cbw + 15, cdb, cdb_len);
    esp_err_t err = msc_bulk(msc_out, cbw, sizeof cbw, NULL);
    size_t got = 0;
    if (err == ESP_OK && len) err = msc_bulk(msc_in, data, len, &got);
    uint8_t csw[MSC_CSW_BYTES] = {0};
    size_t csw_got = 0;
    if (err == ESP_OK) err = msc_bulk(msc_in, csw, sizeof csw, &csw_got);
    if (err == ESP_OK) {
        uint32_t signature, csw_tag;
        memcpy(&signature, csw, 4);
        memcpy(&csw_tag, csw + 4, 4);
        if (csw_got != MSC_CSW_BYTES || signature != MSC_CSW_SIGNATURE || csw_tag != tag) err = ESP_ERR_INVALID_RESPONSE;
        else if (csw[12] != 0) err = ESP_ERR_INVALID_STATE; /* command failed: sense data says why */
        else if (len && got != len) err = ESP_ERR_INVALID_SIZE;
    }
    if (err != ESP_OK) msc_errors++;
    return err;
}

/* Owner task: waits for a stick, makes it ready and returns its 512-byte sector count. */
esp_err_t ruzina_msc_open(uint32_t *sectors)
{
    while (!msc_bound) vTaskDelay(pdMS_TO_TICKS(100));
    xSemaphoreTake(msc_bound, portMAX_DELAY);
    uint8_t buffer[36];
    const uint8_t inquiry[6] = {0x12, 0, 0, 0, sizeof buffer, 0};
    esp_err_t err = msc_command(inquiry, sizeof inquiry, buffer, sizeof buffer);
    if (err != ESP_OK) return err;
    printf("usb_msc inquiry type=%02x removable=%u vendor=\"%.8s\" product=\"%.16s\"\n", buffer[0] & 0x1f,
           buffer[1] >> 7, (const char *)buffer + 8, (const char *)buffer + 16);
    /* A stick may need a moment after attach: TEST UNIT READY until it reports ready. */
    const uint8_t ready[6] = {0};
    uint8_t sense[18];
    const uint8_t request_sense[6] = {0x03, 0, 0, 0, sizeof sense, 0};
    for (int attempt = 0;; attempt++) {
        err = msc_command(ready, sizeof ready, NULL, 0);
        if (err == ESP_OK) break;
        if (err != ESP_ERR_INVALID_STATE || attempt >= 20) return err;
        msc_command(request_sense, sizeof request_sense, sense, sizeof sense);
        vTaskDelay(pdMS_TO_TICKS(100));
    }
    uint8_t capacity[8];
    const uint8_t read_capacity[10] = {0x25};
    err = msc_command(read_capacity, sizeof read_capacity, capacity, sizeof capacity);
    if (err != ESP_OK) return err;
    const uint32_t last = (uint32_t)capacity[0] << 24 | capacity[1] << 16 | capacity[2] << 8 | capacity[3];
    const uint32_t block = (uint32_t)capacity[4] << 24 | capacity[5] << 16 | capacity[6] << 8 | capacity[7];
    printf("usb_msc capacity last_lba=%" PRIu32 " block_bytes=%" PRIu32 "\n", last, block);
    fflush(stdout);
    /* The media stack reads 512-byte sectors; READ CAPACITY(16) sticks (2 TiB+) are out. */
    if (block != 512 || last == 0xffffffffu) return ESP_ERR_NOT_SUPPORTED;
    *sectors = last + 1;
    return ESP_OK;
}

/* Owner task: reads `count` 512-byte sectors from `first` into `data`. */
esp_err_t ruzina_msc_read(uint32_t first, uint32_t count, uint8_t *data)
{
    const uint32_t len = count * 512u;
    if (count == 0 || len > MSC_MAX_DATA_BYTES || count > 0xffff) return ESP_ERR_INVALID_ARG;
    const uint8_t read10[10] = {0x28, 0, (uint8_t)(first >> 24), (uint8_t)(first >> 16), (uint8_t)(first >> 8),
                                (uint8_t)first, 0, (uint8_t)(count >> 8), (uint8_t)count, 0};
    return msc_command(read10, sizeof read10, data, len);
}
