#include <inttypes.h>
#include <stdio.h>
#include "esp_err.h"
#include "esp_intr_alloc.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "usb/usb_host.h"

/* Isolated USB host enumeration diagnostic on the HS (UTMI) port. It logs the
 * device, configuration, interface and endpoint descriptors of each attached
 * device and its detach, and claims no interface or transfer. VBUS on this
 * board is hard-wired to USB5V_IN (see docs/boards/JC1060_USB_TOPOLOGY.md);
 * the stack's root-port power control does not switch it. */

static usb_host_client_handle_t client;
static usb_device_handle_t device;
static volatile uint8_t pending_address;
static volatile bool gone;
static uint32_t attaches, detaches;

static void lib_task(void *unused)
{
    (void)unused;
    for (;;) {
        uint32_t flags = 0;
        usb_host_lib_handle_events(portMAX_DELAY, &flags);
        if (flags & USB_HOST_LIB_EVENT_FLAGS_ALL_FREE) printf("usb_host all_free=1\n");
    }
}

static void on_event(const usb_host_client_event_msg_t *msg, void *arg)
{
    (void)arg;
    if (msg->event == USB_HOST_CLIENT_EVENT_NEW_DEV) pending_address = msg->new_dev.address;
    else if (msg->event == USB_HOST_CLIENT_EVENT_DEV_GONE) gone = true;
}

/* Prints a string descriptor's UTF-16LE code units as ASCII ('?' otherwise). */
static void print_string(const char *name, const usb_str_desc_t *desc)
{
    printf(" %s=\"", name);
    if (desc)
        for (int i = 0; i < (desc->bLength - 2) / 2; ++i) {
            uint16_t c = desc->wData[i];
            putchar(c >= 0x20 && c < 0x7f && c != '"' ? (int)c : '?');
        }
    putchar('"');
}

static void report(uint8_t address)
{
    const usb_device_desc_t *dev;
    const usb_config_desc_t *config;
    usb_device_info_t info;
    esp_err_t err = usb_host_device_open(client, address, &device);
    if (err == ESP_OK) err = usb_host_device_info(device, &info);
    if (err == ESP_OK) err = usb_host_get_device_descriptor(device, &dev);
    if (err == ESP_OK) err = usb_host_get_active_config_descriptor(device, &config);
    if (err != ESP_OK) {
        printf("usb_attach_error address=%u error=%s\n", address, esp_err_to_name(err));
        return;
    }
    static const char *const speeds[] = {"LOW", "FULL", "HIGH"};
    printf("usb_attach n=%" PRIu32 " address=%u speed=%s usb=%04x vid=%04x pid=%04x class=%02x/%02x/%02x "
           "mps0=%u configs=%u config=%u total=%u interfaces=%u max_power_ma=%u",
           ++attaches, info.dev_addr, info.speed <= USB_SPEED_HIGH ? speeds[info.speed] : "?", dev->bcdUSB,
           dev->idVendor, dev->idProduct, dev->bDeviceClass, dev->bDeviceSubClass, dev->bDeviceProtocol,
           dev->bMaxPacketSize0, dev->bNumConfigurations, config->bConfigurationValue, config->wTotalLength,
           config->bNumInterfaces, config->bMaxPower * 2u);
    print_string("manufacturer", info.str_desc_manufacturer);
    print_string("product", info.str_desc_product);
    printf("\n");
    /* Walk the whole configuration: interfaces, endpoints and class-specific
     * descriptors (type 0x24 audio/MIDI interface, 0x25 endpoint). */
    const uint8_t *p = (const uint8_t *)config, *end = p + config->wTotalLength;
    unsigned interfaces = 0, endpoints = 0, class_specific = 0;
    for (p += config->bLength; p + 2 <= end && p[0] >= 2 && p + p[0] <= end; p += p[0]) {
        if (p[1] == 0x04 && p[0] >= 9) {
            interfaces++;
            printf("usb_iface number=%u alt=%u endpoints=%u class=%02x/%02x/%02x\n", p[2], p[3], p[4], p[5], p[6], p[7]);
        } else if (p[1] == 0x05 && p[0] >= 7) {
            endpoints++;
            printf("usb_ep address=%02x attributes=%02x mps=%u interval=%u\n", p[2], p[3], p[4] | p[5] << 8, p[6]);
        } else if (p[1] == 0x0b && p[0] >= 8) {
            printf("usb_iad first=%u count=%u class=%02x/%02x/%02x\n", p[2], p[3], p[4], p[5], p[6]);
        } else if (p[1] == 0x24 || p[1] == 0x25) {
            class_specific++;
        }
    }
    printf("usb_config_walk interfaces=%u endpoints=%u class_specific=%u complete=%u\n",
           interfaces, endpoints, class_specific, p == end);
    fflush(stdout);
}

static void client_task(void *unused)
{
    (void)unused;
    usb_host_client_config_t config = {
        .is_synchronous = false,
        .max_num_event_msg = 4,
        .async = {.client_event_callback = on_event, .callback_arg = NULL},
    };
    ESP_ERROR_CHECK(usb_host_client_register(&config, &client));
    for (;;) {
        usb_host_client_handle_events(client, portMAX_DELAY);
        if (pending_address) {
            uint8_t address = pending_address;
            pending_address = 0;
            if (!device) report(address);
        }
        if (gone && device) {
            gone = false;
            usb_host_device_close(client, device);
            device = NULL;
            printf("usb_detach n=%" PRIu32 "\n", ++detaches);
            fflush(stdout);
        }
    }
}

esp_err_t ruzina_usb_host_enum_start(void)
{
    /* peripheral_map 0 selects the default, high-speed (UTMI) peripheral. */
    const usb_host_config_t host = {.skip_phy_setup = false, .intr_flags = ESP_INTR_FLAG_LEVEL1};
    esp_err_t err = usb_host_install(&host);
    if (err != ESP_OK) return err;
    if (xTaskCreatePinnedToCore(lib_task, "usb_lib", 4096, NULL, 5, NULL, 0) != pdPASS ||
        xTaskCreatePinnedToCore(client_task, "usb_client", 4096, NULL, 4, NULL, 0) != pdPASS)
        return ESP_ERR_NO_MEM;
    printf("S6_USB_HOST port=HS phy=UTMI claim=NONE vbus=HARDWIRED_USB5V_IN\n");
    fflush(stdout);
    return ESP_OK;
}
