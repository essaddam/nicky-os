use serde_json::Value;
use std::process::Command;

#[test]
fn executable_renders_and_reports_both_destinations_without_network_or_audio() {
    let output = Command::new(env!("CARGO_BIN_EXE_moli-music-engine-poc"))
        .arg("demo")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    for destination in ["null", "wav"] {
        assert_eq!(report[destination]["frames"], 9600);
        assert_eq!(report[destination]["samples"], 19200);
        assert_eq!(report[destination]["pcm_exact"], true);
        assert_eq!(report[destination]["physical_outputs"], 0);
        assert_eq!(report[destination]["network_clients"], 0);
    }
    assert_eq!(report["null"]["pcm_sha256"], report["wav"]["pcm_sha256"]);
    assert_eq!(report["wav"]["wav_file_bytes"], 38444);
    assert_eq!(report["automatic_playback_on_create"], false);
    assert_eq!(report["full_music_engine"], false);
}

#[test]
fn executable_refuses_arbitrary_source_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_moli-music-engine-poc"))
        .arg("https://example.com/unapproved-source")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}
