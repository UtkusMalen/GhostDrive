use ghostdrive_indexer::FileIndex;
use ghostdrive_core::{FileMetadata, MediaHash};
use std::path::PathBuf;

#[test]
fn test_crud_operations() {
    let temp_dir = std::env::temp_dir().join("db_crud_test");
    let _ = std::fs::remove_dir_all(&temp_dir);
    let db_path = temp_dir.join("test_crud.db");

    let mut db = FileIndex::open(db_path.clone()).unwrap();

    let meta = FileMetadata {
        path: PathBuf::from("/test/video.mp4"),
        hash: MediaHash("abc123456".into()),
        size: 1024,
        mime_type: "video/mp4".into(),
        created_at: 1234567890,
    };

    // Upsert
    db.upsert_file(&meta).unwrap();

    // Get by path
    let retrieved = db.get_by_path(&meta.path).unwrap().unwrap();
    assert_eq!(retrieved, meta);

    // Get by hash
    let retrieved_hash = db.get_by_hash(&meta.hash).unwrap().unwrap();
    assert_eq!(retrieved_hash, meta);

    // List
    let all = db.list_all().unwrap();
    assert_eq!(all.len(), 1);

    // Remove
    db.remove_file(&meta.path).unwrap();
    assert!(db.get_by_path(&meta.path).unwrap().is_none());
    assert!(db.get_by_hash(&meta.hash).unwrap().is_none());

    // Compact
    let _ = db.compact().unwrap();

    // Cleanup
    let _ = std::fs::remove_dir_all(temp_dir);
}