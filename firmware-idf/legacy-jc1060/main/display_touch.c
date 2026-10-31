#include <inttypes.h>
#include <stdio.h>
#include <string.h>
#include "driver/gpio.h"
#include "driver/i2c_master.h"
#include "driver/ledc.h"
#include "esp_cache.h"
#include "esp_heap_caps.h"
#include "esp_lcd_mipi_dsi.h"
#include "esp_lcd_panel_io.h"
#include "esp_lcd_panel_ops.h"
#include "esp_ldo_regulator.h"
#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "sdkconfig.h"
#include "new_panel_commands.h"

/* Single display owner. GPIO21/22 remain untouched; no audio/C6 owner. The
 * diagnostic profile adds touch; the load profile shares only the panel. */
static esp_ldo_channel_handle_t phy_power;
static esp_lcd_dsi_bus_handle_t bus;
static esp_lcd_panel_io_handle_t io;
static esp_lcd_panel_handle_t panel;
static uint16_t *frame;
static bool pwm_ready;

static esp_err_t brightness(unsigned duty)
{
    esp_err_t err = ledc_set_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_0, duty);
    return err == ESP_OK ? ledc_update_duty(LEDC_LOW_SPEED_MODE, LEDC_CHANNEL_0) : err;
}

/* Brings up the New_Panel with one driver-owned RGB565 framebuffer. `fill`
 * paints it before scanout starts; the backlight stays off until then. On
 * failure the backlight is off and handles are retained until app restart. */
esp_err_t ruzina_panel_start(void (*fill)(uint16_t *frame), uint16_t **frame_out)
{
    esp_err_t err;
#define TRY(call) do { err = (call); if (err != ESP_OK) goto fail; } while (0)
    ledc_timer_config_t timer = {.speed_mode=LEDC_LOW_SPEED_MODE, .duty_resolution=LEDC_TIMER_10_BIT,
        .timer_num=LEDC_TIMER_1, .freq_hz=20000, .clk_cfg=LEDC_AUTO_CLK};
    TRY(ledc_timer_config(&timer));
    ledc_channel_config_t pwm = {.gpio_num=23, .speed_mode=LEDC_LOW_SPEED_MODE,
        .channel=LEDC_CHANNEL_0, .timer_sel=LEDC_TIMER_1, .duty=0};
    TRY(ledc_channel_config(&pwm)); pwm_ready = true;
    esp_ldo_channel_config_t power = {.chan_id=3, .voltage_mv=2500};
    TRY(esp_ldo_acquire_channel(&power, &phy_power));
    esp_lcd_dsi_bus_config_t dsi = {.bus_id=0, .num_data_lanes=2, .phy_clk_src=MIPI_DSI_PHY_CLK_SRC_DEFAULT,
        .lane_bit_rate_mbps=750};
    TRY(esp_lcd_new_dsi_bus(&dsi, &bus));
    esp_lcd_dbi_io_config_t dbi = {.virtual_channel=0, .lcd_cmd_bits=8, .lcd_param_bits=8};
    TRY(esp_lcd_new_panel_io_dbi(bus, &dbi, &io));
    gpio_config_t reset = {.pin_bit_mask=1ULL << 0, .mode=GPIO_MODE_OUTPUT};
    TRY(gpio_config(&reset));
    TRY(gpio_set_level(0, 1)); vTaskDelay(pdMS_TO_TICKS(10));
    TRY(gpio_set_level(0, 0)); vTaskDelay(pdMS_TO_TICKS(10));
    TRY(gpio_set_level(0, 1)); vTaskDelay(pdMS_TO_TICKS(120));
    for (unsigned i=0; i<sizeof(new_panel_commands)/sizeof(new_panel_commands[0]); ++i) {
        const panel_command_t *cmd = &new_panel_commands[i];
        TRY(esp_lcd_panel_io_tx_param(io, cmd->command, cmd->data, cmd->length));
        if (cmd->delay_ms) vTaskDelay(pdMS_TO_TICKS(cmd->delay_ms));
    }
    esp_lcd_dpi_panel_config_t dpi = {.virtual_channel=0, .dpi_clk_src=MIPI_DSI_DPI_CLK_SRC_DEFAULT,
        .dpi_clock_freq_mhz=52, .in_color_format=LCD_COLOR_FMT_RGB565,
        .out_color_format=LCD_COLOR_FMT_RGB565, .num_fbs=1,
        .video_timing={.h_size=1024,.v_size=600,.hsync_back_porch=136,.hsync_pulse_width=24,
            .hsync_front_porch=160,.vsync_back_porch=21,.vsync_pulse_width=2,.vsync_front_porch=12}};
    TRY(esp_lcd_new_panel_dpi(bus, &dpi, &panel));
    void *frame_storage = NULL;
    TRY(esp_lcd_dpi_panel_get_frame_buffer(panel, 1, &frame_storage));
    frame = frame_storage;
    fill(frame);
    TRY(esp_cache_msync(frame, 1024*600*2, ESP_CACHE_MSYNC_FLAG_DIR_C2M));
    TRY(esp_lcd_panel_init(panel));
    TRY(brightness(256));
    printf("S6_DISPLAY_INIT commands=60 lanes=2 mbps=750 pixel_mhz=52 size=1024x600 format=RGB565 framebuffer_bytes=1228800 backlight_percent=25\n");
    *frame_out = frame;
    return ESP_OK;
fail:
    printf("s6_error=%s backlight=OFF\n", esp_err_to_name(err));
    if (pwm_ready) brightness(0);
    return err;
#undef TRY
}

#if CONFIG_RUZINA_DISPLAY_TOUCH
static i2c_master_bus_handle_t i2c;
static i2c_master_dev_handle_t touch;
static uint16_t touch_width, touch_height;

static esp_err_t read_touch(uint16_t reg, uint8_t *data, size_t size)
{
    uint8_t address[2] = {reg >> 8, reg & 255};
    return i2c_master_transmit_receive(touch, address, 2, data, size, 100);
}

static void touch_task(void *unused)
{
    (void)unused;
    uint32_t samples = 0, errors = 0;
    int64_t report = 0;
    for (;;) {
        uint8_t status;
        esp_err_t err = read_touch(0x814e, &status, 1);
        if (err == ESP_OK && (status & 0x80)) {
            unsigned count = status & 15;
            uint8_t points[40];
            if (count > 5) { errors++; count = 0; }
            if (count && read_touch(0x814f, points, count * 8) == ESP_OK) {
                unsigned x = points[1] | points[2] << 8;
                unsigned y = points[3] | points[4] << 8;
                /* Input dimensions come from the chip's current RAM config,
                 * never assumed from a stale 800x480 BSP calibration message. */
                unsigned mapped_x = x * 1023u / (touch_width - 1u);
                unsigned mapped_y = y * 599u / (touch_height - 1u);
                if (mapped_x > 1023) mapped_x = 1023;
                if (mapped_y > 599) mapped_y = 599;
                printf("touch_sample=%" PRIu32 " count=%u raw=%u,%u mapped=%u,%u\n",
                       ++samples, count, x, y, mapped_x, mapped_y);
            } else if (count) errors++;
            /* Acknowledge only the ready status byte, never persistent config. */
            const uint8_t clear[] = {0x81, 0x4e, 0};
            if (i2c_master_transmit(touch, clear, sizeof(clear), 100) != ESP_OK) errors++;
        } else if (err != ESP_OK) errors++;
        if (esp_timer_get_time() - report > 5000000) {
            printf("touch_health samples=%" PRIu32 " errors=%" PRIu32 "\n", samples, errors);
            report = esp_timer_get_time();
        }
        vTaskDelay(pdMS_TO_TICKS(40));
    }
}

/* Fill exactly one driver-owned framebuffer before scanout starts. No
 * active-frame CPU writes, alternate owner, or source DMA buffer lifetime. */
static void stripes(uint16_t *fb)
{
    const uint16_t colors[] = {0xf800,0x07e0,0x001f,0xffff,0x0000,0xffe0,0x07ff,0xf81f};
    for (unsigned y=0; y<600; ++y) {
        for (unsigned x=0; x<1024; ++x) {
            bool marker=(x<32&&y<32)||(x>=992&&y<32)||(x<32&&y>=568)||(x>=992&&y>=568);
            fb[y*1024+x] = marker ? 0xffff : colors[x/128];
        }
    }
}

esp_err_t ruzina_display_touch_start(void)
{
    esp_err_t err;
#define TRY(call) do { err = (call); if (err != ESP_OK) goto fail; } while (0)
    uint16_t *fb;
    err = ruzina_panel_start(stripes, &fb);
    if (err != ESP_OK) return err;

    i2c_master_bus_config_t wire = {.i2c_port=0, .sda_io_num=7, .scl_io_num=8,
        .clk_source=I2C_CLK_SRC_DEFAULT, .glitch_ignore_cnt=7, .flags.enable_internal_pullup=true};
    TRY(i2c_new_master_bus(&wire, &i2c));
    uint16_t address = 0;
    if (i2c_master_probe(i2c, 0x5d, 100) == ESP_OK) address=0x5d;
    else if (i2c_master_probe(i2c, 0x14, 100) == ESP_OK) address=0x14;
    if (!address) { err=ESP_ERR_NOT_FOUND; goto fail; }
    i2c_device_config_t dev = {.dev_addr_length=I2C_ADDR_BIT_LEN_7, .device_address=address, .scl_speed_hz=100000};
    TRY(i2c_master_bus_add_device(i2c, &dev, &touch));
    uint8_t id[4], dimensions[4];
    TRY(read_touch(0x8140, id, sizeof(id)));
    if (memcmp(id, "911", 3) != 0) { err=ESP_ERR_INVALID_RESPONSE; goto fail; }
    TRY(read_touch(0x8048, dimensions, sizeof(dimensions)));
    touch_width=dimensions[0] | dimensions[1]<<8; touch_height=dimensions[2] | dimensions[3]<<8;
    if (touch_width<2 || touch_height<2 || touch_width>4096 || touch_height>4096) {err=ESP_ERR_INVALID_RESPONSE;goto fail;}
    printf("S6_TOUCH_ID address=0x%02x id=%02x%02x%02x%02x config_size=%ux%u mode=POLLING reset_irq=UNTOUCHED\n",
           address,id[0],id[1],id[2],id[3],touch_width,touch_height);
    if (xTaskCreate(touch_task,"s6_touch",4096,NULL,1,NULL)!=pdPASS) {err=ESP_ERR_NO_MEM;goto fail;}
    printf("s6_readiness=AWAITING_OPERATOR_COLORS_CORNERS_AND_TOUCH\n");
    return ESP_OK;
fail:
    printf("s6_error=%s backlight=OFF\n", esp_err_to_name(err));
    if (pwm_ready) brightness(0);
    /* Retain failed peripheral handles for diagnostics until controlled app
     * restart. No concurrent task exists on these initialization-failure paths. */
    return err;
#undef TRY
}
#endif
