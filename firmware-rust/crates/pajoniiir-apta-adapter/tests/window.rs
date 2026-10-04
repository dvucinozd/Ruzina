use libapta::{
    FrameRange, WaveformColumn, WaveformSpan,
    detail_analysis::DetailTile,
    session::{CancellationToken, SessionConfig, WorkBudget},
    sparse::{NODE_FRAMES, QueuedBlock, SparseAccumulator, SparseSession, Workspace},
    waveform::{NormalizedSample, PcmView},
};
use pajoniiir_apta_adapter::{Error, window};
use pajoniiir_waveform::{Rgb565Surface, WaveformColumn as RenderColumn};
const EMPTY: RenderColumn = RenderColumn {
    peak: 42,
    color_rgb565: 42,
};

#[test]
fn real_sparse_pcm_and_detail_windows_render_without_bridging_holes() {
    let mut output = [EMPTY; 2];
    let view = {
        let mut accumulators = vec![SparseAccumulator::default(); 128];
        let mut ranges = [FrameRange {
            first_frame: 0,
            end_frame: 0,
        }; 4];
        let mut nodes = [QueuedBlock::default(); 2];
        let mut pcm = vec![NormalizedSample::default(); 2 * NODE_FRAMES];
        let mut spans = vec![WaveformSpan::default(); 128];
        let mut columns = vec![WaveformColumn::default(); 128];
        let mut detail = vec![DetailTile::default(); 4];
        let mut session = SparseSession::new(
            SessionConfig {
                sample_rate: 8000,
                channel_count: 1,
                total_frames: 32768,
                frames_per_column: 256,
            },
            Workspace {
                accumulators: &mut accumulators,
                ranges: &mut ranges,
                nodes: &mut nodes,
                pcm: &mut pcm,
                snapshot_spans: &mut spans,
                snapshot_columns: &mut columns,
            },
        )
        .unwrap();
        session.enable_detail(&mut detail[..]).unwrap();
        // First two columns cross the tile boundary; later data leaves a hole.
        for (frame, amplitude) in [(16128, 8192i16), (16384, -16384), (17408, 4096)] {
            assert_eq!(
                session.push_at(frame, PcmView::S16Interleaved(&[amplitude; 256])),
                Ok(256)
            );
            session
                .process(WorkBudget::default(), &CancellationToken::new())
                .unwrap();
        }
        session.finish_input().unwrap();
        session
            .process(WorkBudget::default(), &CancellationToken::new())
            .unwrap();
        let snapshot = session.snapshot_graph(1).unwrap();
        let input = snapshot.view();
        let range = FrameRange {
            first_frame: 16128,
            end_frame: 16640,
        };
        let detail = input.detail.unwrap();
        let overview = input.overview.unwrap();
        let mut overview_output = [EMPTY; 2];
        let overview_view =
            window::overview(overview, range, &mut overview_output, 0x07e0).unwrap();
        let detail_view = window::detail(detail, range, &mut output, 0x07e0).unwrap();
        assert_eq!(detail_view.columns, overview_view.columns);
        assert_eq!(detail_view.columns[0].peak, 16384);
        assert_eq!(detail_view.columns[1].peak, 32769);
        let mut untouched = [EMPTY; 6];
        let gap = FrameRange {
            first_frame: 16128,
            end_frame: 17664,
        };
        assert!(matches!(
            window::overview(overview, gap, &mut untouched, 1),
            Err(Error::UnsupportedGrid)
        ));
        assert_eq!(untouched, [EMPTY; 6]);
        assert!(matches!(
            window::detail(detail, gap, &mut untouched, 1),
            Err(Error::UnsupportedGrid)
        ));
        assert_eq!(untouched, [EMPTY; 6]);
        assert!(matches!(
            window::detail(detail, range, &mut [], 1),
            Err(Error::Capacity)
        ));
        assert!(matches!(
            window::detail(
                detail,
                FrameRange {
                    first_frame: 16129,
                    end_frame: 16640
                },
                &mut untouched,
                1
            ),
            Err(Error::Coordinate)
        ));
        detail_view
    };
    // Retain independent converted data after both sparse session and cache drop.
    let mut pixels = [0; 32 * 16];
    Rgb565Surface::new(&mut pixels, 32, 16)
        .unwrap()
        .draw_centered_waveform(0, 0, 32, 16, view);
    assert!(pixels.contains(&0x07e0));
}
