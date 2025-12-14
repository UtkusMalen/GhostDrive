use ghostdrive_network::StreamNode;

#[tokio::test]
async fn test_persistent_identity() {
    let temp_dir = std::env::temp_dir().join("ghostdrive_test_node");
    let _ = tokio::fs::remove_dir_all(&temp_dir).await; // Cleanup prev runs

    // First run: Generate key
    let node1 = StreamNode::new(temp_dir.clone()).await.unwrap();
    let id1 = node1.node_id();
    let relay1 = node1.relay_url();
    println!("Run 1 - Node ID: {}, Relay: {}", id1, relay1);

    // Simulate restart
    drop(node1);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Second run: Load key
    let node2 = StreamNode::new(temp_dir.clone()).await.unwrap();
    let id2 = node2.node_id();
    println!("Run 2 - Node ID: {}", id2);

    assert_eq!(id1, id2, "Node ID should persist across restarts");
    assert!(temp_dir.join("secret.key").exists());

    // Cleanup
    let _ = tokio::fs::remove_dir_all(temp_dir).await;
}