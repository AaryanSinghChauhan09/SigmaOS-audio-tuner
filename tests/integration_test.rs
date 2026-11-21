// Integration test for SigmaOS-audio-tuner
// Verifies the crate builds and links as a dependency.
#![allow(unused_imports)]

use audio_tuner as _;

#[test]
fn crate_links_and_runs() {
    assert_eq!(2 + 2, 4);
}
