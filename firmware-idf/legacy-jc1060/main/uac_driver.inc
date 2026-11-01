#include "uac_timing.h"
/* Rust (controller_audio.rs); every call runs on the client task. */
extern int32_t ruzina_legacy_uac_select(const uint8_t *config, uint32_t len, int32_t high_speed, uint32_t *out);
extern uint32_t ruzina_legacy_uac_control(uint32_t step, uint8_t *out);
extern uint32_t ruzina_legacy_uac_fill(uint8_t *buffer, uint32_t capacity, uint32_t *sizes, uint32_t packets);
extern void ruzina_legacy_uac_stats(uint32_t *out);
#if CONFIG_RUZINA_DECK_PLAY
extern void ruzina_legacy_play_diagnostics(uint32_t *out);
#endif
#if CONFIG_RUZINA_DECK_MIX
#include "esp_timer.h"
/* Rust (deck_mix.rs): rendered frames, underrun blocks per deck, buffered
 * frames and rate per deck, limited MAIN samples. */
extern void ruzina_legacy_mix_stats(uint32_t *out);
#if CONFIG_RUZINA_DECK_ENGINE
/* Rust (deck_engine.rs, deck_mix.rs): position ms and flags per deck;
 * unexecuted effects and refused loads. */
extern void ruzina_legacy_engine_status(uint32_t *out);
extern void ruzina_legacy_engine_counters(uint32_t *out);
#endif
/* Longest Rust fill of one transfer (six 1 ms packets of both decks). */
static uint32_t uac_fill_max_us;
static uac_fill_timing_t uac_fill_timing;
static bool uac_any_iso_submitted;
#elif CONFIG_RUZINA_DECK_PLAY
/* Rust (deck_play.rs): played frames, underrun packets, buffered frames, rate. */
extern void ruzina_legacy_play_stats(uint32_t *out);
#endif

/* Four isochronous OUT transfers of six 1 ms packets keep 24 ms queued. At
 * high speed a 1 ms bInterval is 8 microframes, and esp-usb fits at most
 * (64 - 3) microframes per ISOC transfer, so 7 packets is the hard ceiling:
 * 8 failed on the board with ESP_ERR_INVALID_SIZE. */
#define UAC_TRANSFERS 4
#define UAC_PACKETS 6
enum { UAC_OFF = -1, UAC_SET_ALT, UAC_SET_RATE, UAC_STREAMING };
static usb_transfer_t *uac_ctrl, *uac_iso[UAC_TRANSFERS];
static bool uac_ctrl_busy, uac_iso_busy[UAC_TRANSFERS];
static uint8_t uac_interface = 0xff;
static int uac_step = UAC_OFF;
static uint32_t uac_info[8]; /* interface, alt, endpoint, mps, channels, bytes, rate, freq_control */
static uint32_t uac_completed, uac_iso_errors, uac_packet_errors, uac_ctrl_errors, uac_fill_errors;
static uint32_t uac_submit_errors, uac_generation;
static uac_recovery_t uac_health;
static bool uac_report_pending;
typedef struct { uint32_t generation; unsigned slot; } uac_context_t;
static uac_context_t uac_context[UAC_TRANSFERS], uac_control_context;
static void uac_start_stream(enum uac_fill_phase phase);

static bool uac_refill(usb_transfer_t *transfer, enum uac_fill_phase phase)
{
    uint32_t sizes[UAC_PACKETS];
#if CONFIG_RUZINA_DECK_MIX
    int64_t start = esp_timer_get_time();
#endif
    uint32_t bytes = ruzina_legacy_uac_fill(transfer->data_buffer, (uint32_t)transfer->data_buffer_size,
                                            sizes, UAC_PACKETS);
#if CONFIG_RUZINA_DECK_MIX
    int64_t elapsed = esp_timer_get_time() - start;
    uint32_t took = elapsed > UINT32_MAX ? UINT32_MAX : (uint32_t)elapsed;
    if (took > uac_fill_max_us) uac_fill_max_us = took;
    uac_timing_record(&uac_fill_timing,phase,took,uac_any_iso_submitted,!bytes);
#else
    (void)phase;
#endif
    if (!bytes) { uac_fill_errors++; return false; }
    for (int i = 0; i < UAC_PACKETS; ++i) transfer->isoc_packet_desc[i].num_bytes = (int)sizes[i];
    transfer->num_bytes = (int)bytes;
    return true;
}

static void uac_iso_done(usb_transfer_t *transfer)
{
    const uac_context_t *context = transfer->context;
    if (!uac_recovery_current(&uac_health, context->generation)) return;
    unsigned index = context->slot;
    uac_iso_busy[index] = false;
    if (transfer->status==USB_TRANSFER_STATUS_NO_DEVICE) {
        uac_health.state=UAC_HEALTH_STOPPING; uac_step=UAC_OFF; return;
    }
    if (transfer->status==USB_TRANSFER_STATUS_CANCELED && uac_health.state==UAC_HEALTH_STREAMING)
        uac_recovery_fault(&uac_health);
    if (transfer->status == USB_TRANSFER_STATUS_COMPLETED) {
        uac_completed++;
        for (int i = 0; i < UAC_PACKETS; ++i)
            if (transfer->isoc_packet_desc[i].status != USB_TRANSFER_STATUS_COMPLETED) {
                uac_packet_errors++; uac_recovery_fault(&uac_health);
            }
    } else if (transfer->status != USB_TRANSFER_STATUS_NO_DEVICE
            && transfer->status != USB_TRANSFER_STATUS_CANCELED) {
        uac_iso_errors++; uac_recovery_fault(&uac_health);
    }
    /* Keep the stream running: refill this transfer and queue it again. */
    if (uac_step == UAC_STREAMING && uac_health.state == UAC_HEALTH_STREAMING && device && !gone) {
        if (!uac_refill(transfer,UAC_FILL_CALLBACK)) uac_recovery_fault(&uac_health);
        else if (usb_host_transfer_submit(transfer) == ESP_OK) uac_iso_busy[index] = true;
        else { uac_submit_errors++; uac_recovery_fault(&uac_health); }
    }
    if (uac_completed && uac_completed % 1000 == 0) uac_report_pending = true;
}

/* Single latest snapshot: USB work never blocks on formatted serial output. */
typedef struct {
    uint32_t completed, stats[3], iso_errors, packet_errors, submit_errors, log_drops;
#if CONFIG_RUZINA_DECK_PLAY
    uint32_t diagnostics[4];
#endif
#if CONFIG_RUZINA_DECK_MIX
    uint32_t mix[8], fill_max_us;
    uint32_t generation;
    uac_fill_timing_t timing;
#if CONFIG_RUZINA_DECK_ENGINE
    uint32_t engine[4], counters[2];
#endif
#elif CONFIG_RUZINA_DECK_PLAY
    uint32_t play[4];
#endif
} uac_log_snapshot_t;
static QueueHandle_t uac_log_queue;
static uint32_t uac_log_drops;

static void uac_log_task(void *unused)
{
    (void)unused;
    uac_log_snapshot_t sample;
    for (;;) {
        if (xQueueReceive(uac_log_queue, &sample, portMAX_DELAY) != pdTRUE) continue;
#if CONFIG_RUZINA_DECK_PLAY
        printf("producer_water low_us=%" PRIu32 "/%" PRIu32 " prime_max_us=%" PRIu32 "/%" PRIu32 " log_drops=%" PRIu32 "\n",
            sample.diagnostics[0],sample.diagnostics[1],sample.diagnostics[2],sample.diagnostics[3],sample.log_drops);
#endif
        printf("usb_uac_stats transfers=%" PRIu32 " packets=%" PRIu32 " frames=%" PRIu32 " tone_packets=%" PRIu32
               " iso_errors=%" PRIu32 " packet_errors=%" PRIu32 " submit_errors=%" PRIu32 "\n",
               sample.completed, sample.stats[0], sample.stats[1], sample.stats[2],
               sample.iso_errors, sample.packet_errors, sample.submit_errors);
#if CONFIG_RUZINA_DECK_MIX
        printf("deck_mix_stats rendered=%" PRIu32 " underruns=%" PRIu32 "/%" PRIu32 " buffered=%" PRIu32 "/%" PRIu32
               " rate=%" PRIu32 "/%" PRIu32 " limited=%" PRIu32 " fill_max_us=%" PRIu32 "\n",
               sample.mix[0],sample.mix[1],sample.mix[2],sample.mix[3],sample.mix[4],sample.mix[5],sample.mix[6],sample.mix[7],sample.fill_max_us);
        printf("uac_fill_first generation=%" PRIu32 " seen=%u phase=%u us=%" PRIu32 " after_iso_submit=%u\n",
               sample.generation,(unsigned)sample.timing.first_connection_seen,
               (unsigned)sample.timing.first_connection_phase,sample.timing.first_connection_us,
               (unsigned)sample.timing.first_connection_after_iso_submit);
        static const char *const phase_names[UAC_FILL_PHASE_COUNT]={"initial_prefill","recovery_prefill","callback"};
        for (unsigned i=0;i<UAC_FILL_PHASE_COUNT;++i) {
            const uac_fill_phase_stats_t *p=&sample.timing.phase[i];
            printf("uac_fill_phase phase=%s generation=%" PRIu32 " scope=BOOT count=%" PRIu32
                   " first_us=%" PRIu32 " first_after_iso_submit=%u max_us=%" PRIu32
                   " max_after_iso_submit=%u gt_3000=%" PRIu32 " ge_6000=%" PRIu32 " failures=%" PRIu32 "\n",
                   phase_names[i],sample.generation,p->count,p->first_us,(unsigned)p->first_after_iso_submit,
                   p->max_us,(unsigned)p->max_after_iso_submit,p->gt_3000,p->ge_6000,p->failures);
        }
#if CONFIG_RUZINA_DECK_ENGINE
        printf("deck_engine_stats d1_ms=%" PRIu32 " d1_flags=%" PRIu32 " d2_ms=%" PRIu32 " d2_flags=%" PRIu32
               " unsupported=%" PRIu32 " refused_loads=%" PRIu32 "\n",
               sample.engine[0],sample.engine[1],sample.engine[2],sample.engine[3],sample.counters[0],sample.counters[1]);
#endif
#elif CONFIG_RUZINA_DECK_PLAY
        printf("deck_play_stats played=%" PRIu32 " underruns=%" PRIu32 " buffered=%" PRIu32 " rate=%" PRIu32 "\n",
               sample.play[0],sample.play[1],sample.play[2],sample.play[3]);
#endif
    }
}

static void uac_report(void)
{
    if (!uac_report_pending) return;
    uac_report_pending = false;
    uac_log_snapshot_t sample={.completed=uac_completed,.iso_errors=uac_iso_errors,
        .packet_errors=uac_packet_errors,.submit_errors=uac_submit_errors};
    ruzina_legacy_uac_stats(sample.stats);
    sample.log_drops=uac_log_drops;
#if CONFIG_RUZINA_DECK_PLAY
    ruzina_legacy_play_diagnostics(sample.diagnostics);
#endif
#if CONFIG_RUZINA_DECK_MIX
    ruzina_legacy_mix_stats(sample.mix); sample.fill_max_us=uac_fill_max_us;
    sample.generation=uac_generation; sample.timing=uac_fill_timing;
#if CONFIG_RUZINA_DECK_ENGINE
    ruzina_legacy_engine_status(sample.engine);ruzina_legacy_engine_counters(sample.counters);
#endif
#elif CONFIG_RUZINA_DECK_PLAY
    ruzina_legacy_play_stats(sample.play);
#endif
    if (!uac_log_queue || uxQueueMessagesWaiting(uac_log_queue)) uac_log_drops++;
    if (uac_log_queue) (void)xQueueOverwrite(uac_log_queue,&sample);
}

static void uac_start_stream(enum uac_fill_phase phase)
{
    uint32_t mps = uac_info[3];
    int started = 0;
    /* Allocate the complete pool before publishing any transfer. */
    for (unsigned i = 0; i < UAC_TRANSFERS; ++i) {
        if (!uac_iso[i] && usb_host_transfer_alloc(UAC_PACKETS * mps, UAC_PACKETS, &uac_iso[i]) != ESP_OK) {
            uac_recovery_fault(&uac_health); return;
        }
        uac_iso[i]->device_handle = device;
        uac_iso[i]->bEndpointAddress = (uint8_t)uac_info[2];
        uac_iso[i]->callback = uac_iso_done;
        uac_context[i] = (uac_context_t){.generation=uac_health.generation, .slot=i};
        uac_iso[i]->context = &uac_context[i];
    }
    for (unsigned i = 0; i < UAC_TRANSFERS; ++i) {
        if (!uac_refill(uac_iso[i],phase)) { uac_recovery_fault(&uac_health); break; }
        if (usb_host_transfer_submit(uac_iso[i]) != ESP_OK) {
            uac_submit_errors++; uac_recovery_fault(&uac_health); break;
        }
        uac_iso_busy[i] = true; started++;
#if CONFIG_RUZINA_DECK_MIX
        uac_any_iso_submitted=true;
#endif
    }
    if (started == UAC_TRANSFERS) uac_health.state = UAC_HEALTH_STREAMING;
    printf("usb_uac stream=%s transfers=%d packets_per_transfer=%d generation=%" PRIu32 "\n",
           started == UAC_TRANSFERS ? "STARTED" : "RECOVERING", started, UAC_PACKETS, uac_health.generation);
}

static bool uac_busy(void)
{
    bool busy=uac_ctrl_busy;
    for (unsigned i=0;i<UAC_TRANSFERS;++i) busy |= uac_iso_busy[i];
    return busy;
}

static void uac_recover_poll(void)
{
    if (!device || gone || uac_interface == 0xff || uac_health.state != UAC_HEALTH_RECOVERING) return;
    if (!uac_health.flushed) {
        /* Pinned esp-usb API: halt -> flush -> retire callbacks -> clear. */
        if (usb_host_endpoint_halt(device, (uint8_t)uac_info[2]) != ESP_OK ||
            usb_host_endpoint_flush(device, (uint8_t)uac_info[2]) != ESP_OK) {
            uac_health.state=UAC_HEALTH_FAULT_HELD;
            printf("usb_uac unavailable=ENDPOINT_RECOVERY_FAILED\n"); return;
        }
        uac_health.flushed=true;
        return; /* callbacks are processed by a subsequent client-loop turn */
    }
    if (uac_recovery_retry(&uac_health, gone, uac_busy())) {
        if (usb_host_endpoint_clear(device, (uint8_t)uac_info[2]) != ESP_OK) {
            uac_health.state=UAC_HEALTH_FAULT_HELD;
        } else uac_start_stream(UAC_FILL_RECOVERY_PREFILL);
    }
    if (uac_health.state==UAC_HEALTH_FAULT_HELD)
        printf("usb_uac unavailable=FAULT_HELD attempts=%u submit_errors=%" PRIu32 "\n", uac_health.attempts, uac_submit_errors);
}

static void uac_send(uint32_t step)
{
    uint32_t len = ruzina_legacy_uac_control(step, uac_ctrl->data_buffer);
    uac_ctrl->num_bytes = (int)len;
    uac_ctrl->device_handle = device;
    uac_ctrl->bEndpointAddress = 0;
    if (len && usb_host_transfer_submit_control(client, uac_ctrl) == ESP_OK) uac_ctrl_busy = true;
    else { uac_ctrl_errors++; uac_step = UAC_OFF; uac_health.state=UAC_HEALTH_FAULT_HELD; }
}

/* SET_INTERFACE, then SET_CUR 48 kHz when the endpoint takes it, then stream. */
static void uac_ctrl_done(usb_transfer_t *transfer)
{
    const uac_context_t *context=transfer->context;
    if (!uac_recovery_current(&uac_health,context->generation)) return;
    uac_ctrl_busy = false;
    if (gone || uac_health.state==UAC_HEALTH_STOPPING || uac_health.state==UAC_HEALTH_FAULT_HELD) return;
    if (transfer->status != USB_TRANSFER_STATUS_COMPLETED) {
        uac_ctrl_errors++;
        printf("usb_uac_control step=%d status=%d\n", uac_step, transfer->status);
        uac_step = UAC_OFF; uac_health.state=UAC_HEALTH_FAULT_HELD;
        return;
    }
    if (uac_step == UAC_SET_ALT && uac_info[7]) {
        uac_step = UAC_SET_RATE;
        uac_send(1);
        return;
    }
    if (uac_step == UAC_SET_ALT || uac_step == UAC_SET_RATE) {
        uac_step = UAC_STREAMING;
        uac_start_stream(UAC_FILL_INITIAL_PREFILL);
    }
}

static void uac_start(const usb_config_desc_t *config, usb_speed_t speed)
{
    if (!uac_log_queue) {
        uac_log_queue=xQueueCreate(1,sizeof(uac_log_snapshot_t));
        if (uac_log_queue && xTaskCreatePinnedToCore(uac_log_task,"uac_log",3072,NULL,1,NULL,0)!=pdPASS) {
            vQueueDelete(uac_log_queue); uac_log_queue=NULL;
        }
    }
    if (!ruzina_legacy_uac_select((const uint8_t *)config, config->wTotalLength, speed == USB_SPEED_HIGH, uac_info)) {
        printf("usb_uac select=NONE\n");
        return;
    }
    uac_generation++; if (!uac_generation) uac_generation=1;
#if CONFIG_RUZINA_DECK_MIX
    uac_timing_connection_begin(&uac_fill_timing); uac_any_iso_submitted=false;
#endif
    uac_recovery_begin(&uac_health,uac_generation);
    uac_interface = (uint8_t)uac_info[0];
    esp_err_t err = usb_host_interface_claim(client, device, uac_interface, (uint8_t)uac_info[1]);
    if (err != ESP_OK) { uac_interface=0xff;uac_health.state=UAC_HEALTH_OFF;return; }
    err = usb_host_transfer_alloc(64, 0, &uac_ctrl);
    if (err != ESP_OK) uac_health.state=UAC_HEALTH_FAULT_HELD;
    if (err == ESP_OK) {
        uac_ctrl->callback = uac_ctrl_done;
        uac_control_context=(uac_context_t){.generation=uac_generation};
        uac_ctrl->context=&uac_control_context;
        uac_step = UAC_SET_ALT;
        uac_send(0);
    }
    printf("usb_uac interface=%" PRIu32 " alt=%" PRIu32 " ep=%02" PRIx32 " mps=%" PRIu32 " channels=%" PRIu32
           " bytes=%" PRIu32 " rate=%" PRIu32 " freq_control=%" PRIu32 " headphones=3/4 claim=%s\n",
           uac_info[0], uac_info[1], uac_info[2], uac_info[3], uac_info[4], uac_info[5], uac_info[6], uac_info[7],
           esp_err_to_name(err));
    fflush(stdout);
}

/* Waits for every audio transfer to leave the host, then frees and releases. */
static bool uac_stop(void)
{
    if (uac_interface == 0xff) return true;
    uac_step = UAC_OFF;
    uac_health.state=UAC_HEALTH_STOPPING;
    for (int i = 0; i < 50; ++i) {
        bool busy = uac_ctrl_busy;
        for (int t = 0; t < UAC_TRANSFERS; ++t) busy |= uac_iso_busy[t];
        if (!busy) break;
        usb_host_client_handle_events(client, pdMS_TO_TICKS(10));
    }
    uint32_t stats[3];
    ruzina_legacy_uac_stats(stats);
    bool busy = uac_ctrl_busy;
    for (int t = 0; t < UAC_TRANSFERS; ++t) busy |= uac_iso_busy[t];
    if (busy) {
        uac_health.state=UAC_HEALTH_FAULT_HELD;
        printf("usb_uac unavailable=DRAIN_TIMEOUT resources=HELD\n"); return false;
    }
    if (!busy) {
        for (int t = 0; t < UAC_TRANSFERS; ++t)
            if (uac_iso[t]) {
                if (usb_host_transfer_free(uac_iso[t]) != ESP_OK) {uac_health.state=UAC_HEALTH_FAULT_HELD;return false;}
                uac_iso[t] = NULL;
            }
        if (uac_ctrl) {
            if (usb_host_transfer_free(uac_ctrl) != ESP_OK) {uac_health.state=UAC_HEALTH_FAULT_HELD;return false;}
            uac_ctrl = NULL;
        }
    }
    esp_err_t release = usb_host_interface_release(client, device, uac_interface);
    printf("usb_uac_stop transfers=%" PRIu32 " packets=%" PRIu32 " frames=%" PRIu32 " tone_packets=%" PRIu32
           " iso_errors=%" PRIu32 " packet_errors=%" PRIu32 " control_errors=%" PRIu32 " fill_errors=%" PRIu32
           " freed=%s release=%s\n", uac_completed, stats[0], stats[1], stats[2], uac_iso_errors, uac_packet_errors,
           uac_ctrl_errors, uac_fill_errors, busy ? "NO_STILL_BUSY" : "YES", esp_err_to_name(release));
    if (release != ESP_OK) { uac_health.state=UAC_HEALTH_FAULT_HELD; return false; }
    uac_interface = 0xff; uac_health.state=UAC_HEALTH_OFF;
    uac_completed = uac_iso_errors = uac_packet_errors = uac_ctrl_errors = uac_fill_errors = 0;
    uac_submit_errors=0; return true;
}
