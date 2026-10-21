#include "rekordbox_anlz.h"
#include <inttypes.h>
#include <stdio.h>
static uint64_t byte_hash(uint64_t digest, uint8_t byte) { return (digest ^ byte) * UINT64_C(0x100000001b3); }
static uint64_t little_hash(uint64_t digest, uint32_t value, int bytes) {
    for (int i = 0; i < bytes; ++i) { digest = byte_hash(digest, value & 255); value >>= 8; } return digest;
}
static uint64_t wave_hash(const uint8_t *bytes, size_t len) {
    uint64_t digest = UINT64_C(0xcbf29ce484222325);
    for (size_t i=0; i<len; ++i) {
        uint16_t white = bytes[i] >> 5;
        uint16_t color = ((white*31/7)<<11) | ((white*63/7)<<5) | 31;
        digest = little_hash(digest, (uint32_t)(bytes[i] & 31)*65535/31, 2);
        digest = little_hash(digest, color, 2);
    } return digest;
}
int main(int argc, char **argv) {
    if (argc != 3) return 2;
    anlz_metadata_t data = {0};
    if (anlz_parse_dat(argv[1], &data) != ESP_OK || anlz_parse_ext(argv[2], &data) != ESP_OK) return 3;
    uint64_t beats = UINT64_C(0xcbf29ce484222325);
    for (size_t i=0; i<data.beat_count; ++i) {
        beats = little_hash(beats, data.beats[i].time_ms, 4);
        /* Normalize documented 1..4 wire phase to neutral 0..3 schema. */
        beats = little_hash(beats, data.beats[i].beat_phase - 1, 2);
        beats = little_hash(beats, data.beats[i].bpm_x100, 4);
    }
    printf("{\"beats\":%u,\"preview\":%u,\"detail\":%u,\"cues\":%u,\"bpm_x100\":%u,\"beat_hash\":\"%016" PRIx64 "\",\"preview_hash\":\"%016" PRIx64 "\",\"detail_hash\":\"%016" PRIx64 "\"}\n",
        data.beat_count, data.has_waveform_low ? 400 : 0, (unsigned)data.waveform_high_len, data.cue_count,
        data.beat_count ? data.beats[0].bpm_x100 : 0, beats,
        wave_hash(data.waveform_low, data.has_waveform_low ? 400 : 0), wave_hash(data.waveform_high, data.waveform_high_len));
    anlz_free(&data); return 0;
}
