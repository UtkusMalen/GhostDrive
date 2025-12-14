#[test]
fn test_db_creation() {
    let temp_dir = std::env::temp_dir().join("db_test");
    let db_path = temp_dir.join("test.redb");
    // Ensure cleanup
    let _ = std::fs::remove_file(&db_path);

    let db = ghostdrive_indexer::FileIndex::open(db_path.clone());
    assert!(db.is_ok());
    assert!(db_path.exists());

    // Cleanup
    let _ = std::fs::remove_file(db_path);
}