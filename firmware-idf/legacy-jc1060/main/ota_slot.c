/* The Web Remote's firmware update on the deck's side: its two app slots
 * (partitions-ota.csv) over IDF's OTA API, and the check that keeps a new image.
 *
 * ruzina-ota (rust-core/src/deck_ota.rs) drives the slot through ruzina_ota_begin, _write,
 * _end, _descriptor and _set_boot, from the web task, only after a bundle's signed header
 * has been checked. The slot is erased as it is written (OTA_WITH_SEQUENTIAL_WRITES), so
 * no single call holds the web task for the whole slot's erase.
 *
 * The bootloader rolls back (CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE): a new image boots
 * pending, and if it restarts before it is kept, the bootloader goes back to the previous
 * one. The Wi-Fi task keeps it (ruzina_ota_settle) once the Web Remote's server listens,
 * because the Web Remote is how a later image arrives; it rolls back at once if the
 * server cannot start. */
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "esp_app_desc.h"
#include "esp_ota_ops.h"
#include "esp_partition.h"

/* The slot being written, and whether it is open. Web task only. */
static const esp_partition_t *target;
static esp_ota_handle_t handle;
static bool slot_open;

/* The size of the slot the next update goes to, 0 without one. */
uint32_t ruzina_ota_slot_size(void)
{
    const esp_partition_t *next = esp_ota_get_next_update_partition(NULL);
    return next ? next->size : 0;
}

int ruzina_ota_begin(void)
{
    if (slot_open) {
        esp_ota_abort(handle);
        slot_open = false;
    }
    target = esp_ota_get_next_update_partition(NULL);
    if (target == NULL) return ESP_ERR_NOT_FOUND;
    const esp_err_t err = esp_ota_begin(target, OTA_WITH_SEQUENTIAL_WRITES, &handle);
    slot_open = err == ESP_OK;
    printf("ota begin slot=%s result=%s\n", target->label, esp_err_to_name(err));
    return err;
}

int ruzina_ota_write(const uint8_t *data, size_t len)
{
    return slot_open ? esp_ota_write(handle, data, len) : ESP_ERR_INVALID_STATE;
}

/* Leaves the slot unbootable; nothing once it is closed. */
void ruzina_ota_abort(void)
{
    if (!slot_open) return;
    esp_ota_abort(handle);
    slot_open = false;
    printf("ota abort slot=%s\n", target->label);
}

/* Closes the slot; IDF checks the written image (its segments and appended hash). */
int ruzina_ota_end(void)
{
    if (!slot_open) return ESP_ERR_INVALID_STATE;
    slot_open = false;
    const esp_err_t err = esp_ota_end(handle);
    printf("ota end slot=%s result=%s\n", target->label, esp_err_to_name(err));
    return err;
}

/* The written image's app description: its project name and version. */
int ruzina_ota_descriptor(uint8_t project[32], uint8_t version[32])
{
    if (target == NULL) return ESP_ERR_INVALID_STATE;
    esp_app_desc_t description;
    const esp_err_t err = esp_ota_get_partition_description(target, &description);
    if (err == ESP_OK) {
        memcpy(project, description.project_name, 32);
        memcpy(version, description.version, 32);
    }
    return err;
}

/* The next boot starts the written image, pending until it is kept. */
int ruzina_ota_set_boot(void)
{
    if (target == NULL || slot_open) return ESP_ERR_INVALID_STATE;
    const esp_err_t err = esp_ota_set_boot_partition(target);
    printf("ota boot slot=%s result=%s\n", target->label, esp_err_to_name(err));
    return err;
}

/* The running slot's name, for the SETUP page. */
const char *ruzina_ota_running_slot(void)
{
    const esp_partition_t *running = esp_ota_get_running_partition();
    return running ? running->label : "?";
}

static bool pending(void)
{
    esp_ota_img_states_t state;
    return esp_ota_get_state_partition(esp_ota_get_running_partition(), &state) == ESP_OK &&
           state == ESP_OTA_IMG_PENDING_VERIFY;
}

/* At start: the running slot and whether it waits to be kept. */
void ruzina_ota_report(void)
{
    const esp_app_desc_t *app = esp_app_get_description();
    printf("ota running=%s version=%s pending=%d\n", ruzina_ota_running_slot(), app->version, pending());
}

/* Keeps a pending image once the Web Remote's server listens (`healthy`), or rolls back
 * to the previous image at once when it cannot start. Nothing for a kept image or one
 * written over USB. */
void ruzina_ota_settle(bool healthy)
{
    if (!pending()) return;
    if (healthy) {
        printf("ota keep slot=%s result=%s\n", ruzina_ota_running_slot(),
               esp_err_to_name(esp_ota_mark_app_valid_cancel_rollback()));
        return;
    }
    printf("ota rollback slot=%s\n", ruzina_ota_running_slot());
    /* Restarts into the previous image; returns only if there is none. */
    const esp_err_t err = esp_ota_mark_app_invalid_rollback_and_reboot();
    printf("ota rollback result=%s\n", esp_err_to_name(err));
}
