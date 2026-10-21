use libapta::{
    analysis::{BIN_CAPACITY, OnsetBin},
    waveform::NormalizedSample,
};
use pajoniiir_apta_adapter::{owner::Workspace, storage::LocalTempoBuffers};
pub struct Work {
    queue: Vec<NormalizedSample>,
    columns: Vec<libapta::WaveformColumn>,
    bins: Vec<OnsetBin>,
    flux: Vec<f32>,
    bytes: [u8; 4096],
    pcm: [f32; 512],
}
impl Work {
    pub fn new() -> Self {
        Self {
            queue: vec![NormalizedSample::default(); 17],
            columns: vec![libapta::WaveformColumn::default(); 1250],
            bins: vec![OnsetBin::default(); BIN_CAPACITY],
            flux: vec![0.; BIN_CAPACITY],
            bytes: [0; 4096],
            pcm: [0.; 512],
        }
    }
    pub fn borrow(&mut self) -> Workspace<'_> {
        Workspace {
            local: LocalTempoBuffers {
                queue: &mut self.queue,
                columns: &mut self.columns,
                bins: &mut self.bins,
                flux: &mut self.flux,
            },
            bytes: &mut self.bytes,
            pcm: &mut self.pcm,
        }
    }
}
pub struct Bank {
    beats: Vec<pajoniiir_track_analysis::Beat>,
    waveform: Vec<pajoniiir_waveform::WaveformColumn>,
}
impl Bank {
    pub fn new() -> Self {
        Self {
            beats: vec![
                pajoniiir_track_analysis::Beat {
                    time_ms: 0,
                    phase: 0,
                    bpm_x100: 0
                };
                128
            ],
            waveform: vec![
                pajoniiir_waveform::WaveformColumn {
                    peak: 0,
                    color_rgb565: 0
                };
                1250
            ],
        }
    }
    pub fn borrow(&mut self) -> pajoniiir_apta_adapter::owner::Output<'_> {
        pajoniiir_apta_adapter::owner::Output {
            beats: &mut self.beats,
            waveform: &mut self.waveform,
        }
    }
}
