use std::path::PathBuf;
use tokio::process::Command;
use ghostdrive_transcoder::{Transcoder, TranscodeOptions};
use futures::StreamExt;

/// Helper to generate a dummy test video
async fn ensure_test_video(path: &PathBuf) {
    if path.exists() { return; }
    let _ = Command::new("ffmpeg")
        .args(&[
            "-f", "lavfi", "-i", "testsrc=duration=1:size=640x360:rate=30",
            "-c:v", "libx264", path.to_str().unwrap()
        ])
        .output().await;
}

#[tokio::test]
async fn test_stream_chunks() {
    let temp_dir = std::env::temp_dir().join("ghostdrive_stream_test");
    let _ = tokio::fs::create_dir_all(&temp_dir).await;
    let video_path = temp_dir.join("test_src.mp4");

    ensure_test_video(&video_path).await;

    // Initialize transcoder
    let opts = TranscodeOptions::default();
    let transcoder = Transcoder::new(video_path, opts).await.expect("Failed to init transcoder");

    // Create stream
    // Using a small chunk size to force multiple iterations
    let stream = transcoder.stream_chunks(4096);

    tokio::pin!(stream);

    let mut total_bytes = 0;
    let mut chunk_count = 0;

    // Consume stream
    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res.expect("Stream error");
        total_bytes += chunk.len();
        chunk_count += 1;

        // Sanity check: chunks shouldn't exceed requested size (roughly)
        assert!(chunk.len() <= 4096);
    }

    println!("Streamed {} bytes in {} chunks", total_bytes, chunk_count);

    assert!(total_bytes > 0, "Stream should not be empty");
    assert!(chunk_count > 0, "Should receive at least one chunk");

    // Cleanup
    let _ = tokio::fs::remove_dir_all(temp_dir).await;
}