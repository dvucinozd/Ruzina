use libapta::{
    FrameRange, WaveformColumn, WaveformSpan,
    detail_analysis::DetailTile,
    session::{CancellationToken, SessionConfig, WorkBudget},
    sparse::{NODE_FRAMES, QueuedBlock, SparseAccumulator, SparseSession, Workspace},
    waveform::{NormalizedSample, PcmView},
};
use ruzina_apta_adapter::{Error, window};
use ruzina_waveform::{Rgb565Surface, WaveformColumn as RenderColumn};
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

#[test]
fn real_partial_eof_columns_have_explicit_whole_column_rendering() {
    use libapta::session::Session;
    let mut queue = [NormalizedSample::default(); 256];
    let mut columns = [WaveformColumn::default(); 2];
    let mut tiles = vec![DetailTile::default(); 4];
    let mut s = Session::new(
        SessionConfig {
            sample_rate: 8000,
            channel_count: 1,
            total_frames: 300,
            frames_per_column: 256,
        },
        &mut queue,
        &mut columns,
    )
    .unwrap();
    s.enable_detail(&mut tiles).unwrap();
    assert_eq!(s.push_interleaved(&[0.25; 256]), Ok(256));
    s.process(WorkBudget::default(), &CancellationToken::new())
        .unwrap();
    assert_eq!(s.push_interleaved(&[-0.5; 44]), Ok(44));
    s.finish_input().unwrap();
    s.process(WorkBudget::default(), &CancellationToken::new())
        .unwrap();
    let snapshot = s.snapshot(1).unwrap();
    let input = snapshot.view();
    let overview = input.overview.unwrap();
    let detail = input.detail.unwrap();
    let range = FrameRange {
        first_frame: 0,
        end_frame: 300,
    };
    let mut a = [EMPTY; 2];
    let mut b = [EMPTY; 2];
    assert!(matches!(
        window::overview(overview, range, &mut a, 1),
        Err(Error::Coordinate)
    ));
    assert_eq!(a, [EMPTY; 2]);
    let view = window::overview_with_eof(overview, 300, range, &mut a, 0x07e0).unwrap();
    let d = window::detail_with_eof(detail, 300, range, &mut b, 0x07e0).unwrap();
    assert_eq!(view.columns, d.columns);
    assert_eq!(view.columns[0].peak, 16384);
    assert_eq!(view.columns[1].peak, 32769);
    let mut pixels = [0; 32 * 16];
    Rgb565Surface::new(&mut pixels, 32, 16)
        .unwrap()
        .draw_centered_waveform(0, 0, 32, 16, view);
    assert!(pixels.contains(&0x07e0));
    for (eof, r) in [
        (
            300,
            FrameRange {
                first_frame: 1,
                end_frame: 300,
            },
        ),
        (301, range),
        (299, range),
        (
            300,
            FrameRange {
                first_frame: 0,
                end_frame: 299,
            },
        ),
        (
            300,
            FrameRange {
                first_frame: 0,
                end_frame: 512,
            },
        ),
    ] {
        let mut untouched = [EMPTY; 2];
        assert!(window::overview_with_eof(overview, eof, r, &mut untouched, 1).is_err());
        assert_eq!(untouched, [EMPTY; 2]);
        assert!(window::detail_with_eof(detail, eof, r, &mut untouched, 1).is_err());
        assert_eq!(untouched, [EMPTY; 2]);
    }
    let mut untouched = [EMPTY];
    assert!(matches!(
        window::overview_with_eof(overview, 300, range, &mut untouched, 1),
        Err(Error::Capacity)
    ));
    assert_eq!(untouched, [EMPTY]);
    // EOF permission does not turn a gap into a valid final column.
    let hole = libapta::NativeOverview {
        spans: &overview.spans[..0],
        ..overview
    };
    let mut untouched = [EMPTY; 2];
    assert!(window::overview_with_eof(hole, 300, range, &mut untouched, 1).is_err());
    assert_eq!(untouched, [EMPTY; 2]);
}
