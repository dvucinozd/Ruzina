//! Pure scheduling policy shared by the target owner and slow-media tests.
use crate::deck_play::Ring;
pub(crate) fn order(
    rings: [&Ring; 2],
    playing: [bool; 2],
    pitch: [i16; 2],
    alternate: bool,
) -> [usize; 2] {
    match playing {
        [true, false] => [0, 1],
        [false, true] => [1, 0],
        [true, true] => {
            if rings[0].supply_us(pitch[0]) <= rings[1].supply_us(pitch[1]) {
                [0, 1]
            } else {
                [1, 0]
            }
        }
        _ => {
            if alternate {
                [1, 0]
            } else {
                [0, 1]
            }
        }
    }
}
