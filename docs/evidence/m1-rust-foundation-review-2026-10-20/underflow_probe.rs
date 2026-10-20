#[cfg(test)]
mod tests {
    use pajoniiir_audio_dsp::PcmFrame;
    use pajoniiir_audio_runtime::render::{
        FixedRateRenderGraph, RenderControl, RenderDeckControl, RenderOutputBlock,
    };

    #[test]
    fn starved_24khz_source_must_remain_silent_after_resampler_runs_out() {
        let mut graph = FixedRateRenderGraph::new();
        let mut output = RenderOutputBlock::default();
        let mut deck = RenderDeckControl::idle();
        deck.playing = true;
        deck.source_sample_rate_hz = 24_000;
        let control = RenderControl {
            decks: [deck, RenderDeckControl::idle()],
            master_gain: 1.0,
        };
        let mut remaining = 4;
        let report = graph.render_block(control, &mut output, |index| {
            if index != 0 || remaining == 0 {
                return None;
            }
            remaining -= 1;
            Some(PcmFrame { left: 1000, right: -1000 })
        });
        let repeated: Vec<_> = output.main.iter().enumerate().skip(16)
            .filter(|(_, frame)| **frame != PcmFrame::default())
            .map(|(index, frame)| (index, frame.left, frame.right)).collect();
        eprintln!("underflow_frames={:?}; nonzero frames after exhaustion={repeated:?}", report.underflow_frames);
        assert!(repeated.is_empty(), "source starvation repeated cached audio");
    }
}
