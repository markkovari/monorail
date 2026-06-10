//! Property tests for CSAFE framing (ADR 0003).

use monorail_pm5::csafe::{build_frame, parse_frame, EXT_START_FLAG, STOP_FLAG};
use proptest::prelude::*;

proptest! {
    #[test]
    fn build_then_parse_round_trips(contents in proptest::collection::vec(any::<u8>(), 0..=512)) {
        let frame = build_frame(&contents);
        prop_assert_eq!(parse_frame(&frame).unwrap(), contents);
    }

    #[test]
    fn interior_never_contains_bare_start_or_stop(
        contents in proptest::collection::vec(any::<u8>(), 0..=512)
    ) {
        let frame = build_frame(&contents);
        // Stuffing must escape 0xF0-0xF2 so frame boundaries stay unambiguous;
        // 0xF3 is the escape flag itself and is allowed in the interior.
        let interior = &frame[1..frame.len() - 1];
        prop_assert!(
            interior.iter().all(|&b| !(EXT_START_FLAG..=STOP_FLAG).contains(&b)),
            "bare flag byte in interior: {interior:02x?}"
        );
    }

    #[test]
    fn single_byte_corruption_never_yields_original_contents(
        contents in proptest::collection::vec(any::<u8>(), 1..=64),
        index_seed: usize,
        mask in 1u8..=255,
    ) {
        let mut frame = build_frame(&contents);
        // Corrupt one interior byte (start/stop flags excluded: those fail
        // structurally by definition).
        let index = 1 + index_seed % (frame.len() - 2);
        frame[index] ^= mask;

        // The XOR checksum is weak: a corrupted frame may parse as *some*
        // contents, but never as the original ones.
        if let Ok(parsed) = parse_frame(&frame) {
            prop_assert_ne!(parsed, contents);
        }
    }
}
