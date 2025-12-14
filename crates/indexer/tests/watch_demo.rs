use std::sync::Arc;
use std::time::Duration;
use ghostdrive_indexer::{FileIndex, FileWatcher};
use tokio::time::sleep;

#[tokio::test]
async fn test_watcher_lifecycle() {
    // Setup logging
    let _ = tracing_subscriber::fmt::try_init();

    // Setup temporary paths
    // Use a unique path to avoid conflicts with other tests
    let temp_root = std::env::temp_dir().join("ghostdrive_watch_test_v1");
    let _ = std::fs::remove_dir_all(&temp_root); // Clean start

    let db_path = temp_root.join("index.db");
    let watch_path = temp_root.join("media");

    // Ensure watch directory exists before starting watcher
    std::fs::create_dir_all(&watch_path).expect("Failed to create watch dir");

    // Initialize components
    let index = Arc::new(FileIndex::open(db_path).expect("Failed to open DB"));

    // Watcher needs a clone of the index
    let watcher = FileWatcher::new(index.clone(), vec![watch_path.clone()])
        .expect("Failed to create watcher");

    // 4. Run watcher in background
    tokio::spawn(async move {
        if let Err(e) = watcher.run().await {
            eprintln!("Watcher error: {:?}", e);
        }
    });

    // Give watcher time to start up
    sleep(Duration::from_millis(200)).await;

    // --- TEST CASE 1: Create File ---
    let file_path = watch_path.join("test_video.mp4");
    let content = "dummy video content";
    std::fs::write(&file_path, content).expect("Failed to write file");

    // Wait for debounce (500ms) + processing time
    sleep(Duration::from_secs(1)).await;

    // Verify it exists in DB
    let found = index.get_by_path(&file_path).expect("DB Read failed");
    assert!(found.is_some(), "File was not indexed after creation");
    let metadata = found.unwrap();
    assert_eq!(metadata.size, content.len() as u64);
    assert_eq!(metadata.mime_type, "video/mp4");

    // --- TEST CASE 2: Remove File ---
    std::fs::remove_file(&file_path).expect("Failed to remove file");

    // Wait for debounce + processing
    sleep(Duration::from_secs(1)).await;

    // Verify it is removed from DB
    let found_after = index.get_by_path(&file_path).expect("DB Read failed");
    assert!(found_after.is_none(), "File was not removed from index after deletion");

    // Cleanup
    let _ = std::fs::remove_dir_all(temp_root);
}