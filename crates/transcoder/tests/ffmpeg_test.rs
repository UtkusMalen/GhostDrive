use std::path::PathBuf;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use ghostdrive_transcoder::{Transcoder, TranscodeOptions};

/// Helper to generate a dummy test video if it doesn't exist
async fn ensure_test_video(path: &PathBuf) {
    if path.exists() {
        return;
    }

    println!("Generating dummy video at {:?}", path);
    let status = Command::new("ffmpeg")
        .args(&[
            "-f", "lavfi",
            "-i", "testsrc=duration=3:size=640x360:rate=30",
            "-f", "lavfi",
            "-i", "sine=frequency=1000:duration=3",
            "-c:v", "libx264",
            "-c:a", "aac",
            "-pix_fmt", "yuv420p",
            path.to_str().unwrap()
        ])
        .output()
        .await
        .expect("Failed to run ffmpeg generator");

    assert!(status.status.success(), "Failed to generate test video");
}

#[tokio::test]
async fn test_ffmpeg_spawn() {
    let temp_dir = std::env::temp_dir().join("ghostdrive_transcode_test");
    let _ = tokio::fs::create_dir_all(&temp_dir).await;
    let video_path = temp_dir.join("test_src.mp4");

    ensure_test_video(&video_path).await;

    let opts = TranscodeOptions::default();

    // Start Transcoding
    let mut transcoder = Transcoder::new(video_path.clone(), opts)
        .await
        .expect("Failed to spawn transcoder");

    // Read Stdout
    let mut stdout = transcoder.stdout().expect("Failed to capture stdout");

    // Buffer for reading MPEG-TS chunks
    let mut buffer = [0u8; 188]; // MPEG-TS packets are 188 bytes

    // Read first packet
    let bytes_read = tokio::time::timeout(Duration::from_secs(5), stdout.read_exact(&mut buffer))
        .await
        .expect("Timed out waiting for ffmpeg output")
        .expect("Failed to read from stdout");

    assert_eq!(bytes_read, 188);

    // MPEG-TS sync byte is 0x47 (71 decimal)
    assert_eq!(buffer[0], 0x47, "Stream does not appear to be MPEG-TS");

    println!("Successfully read MPEG-TS packet from FFmpeg pipe");

    // Cleanup: dropping transcoder kills the process
    drop(transcoder);
}