use ghostdrive_host::{HostConfig, HostDaemon};
use ghostdrive_transcoder::TranscodeOptions;

#[tokio::test]
async fn test_daemon_init_and_share() {
    // Setup
    let test_root = std::env::temp_dir().join("ghostdrive_daemon_test");
    let _ = tokio::fs::remove_dir_all(&test_root).await;

    let data_dir = test_root.join("data");
    let media_dir = test_root.join("media");
    tokio::fs::create_dir_all(&data_dir).await.unwrap();
    tokio::fs::create_dir_all(&media_dir).await.unwrap();

    // Create dummy media file
    let file_path = media_dir.join("test.txt");
    tokio::fs::write(&file_path, "Hello World Media").await.unwrap();

    let config = HostConfig {
        data_dir,
        watch_paths: vec![media_dir.clone()],
        transcode_options: TranscodeOptions::default(),
    };

    // Initialize Daemon
    let daemon = HostDaemon::new(config).await.expect("Failed to start daemon");

    assert!(!daemon.node().node_id().is_empty());

    // Test Share File
    let ticket = daemon.share_file(file_path.clone()).await.expect("Failed to share file");
    assert!(ticket.len() > 10);
    println!("Generated Ticket: {}", ticket);

    // Test Share Folder
    let collection_ticket = daemon.share_folder(media_dir).await.expect("Failed to share folder");
    println!("Generated Collection Ticket: {}", collection_ticket);

    // Cleanup
    let _ = tokio::fs::remove_dir_all(test_root).await;
}