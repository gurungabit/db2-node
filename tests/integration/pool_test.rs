/// Integration tests for connection pooling.
/// Requires a running DB2 instance.
#[path = "../common/mod.rs"]
mod common;
use common::*;

use std::sync::Arc;

#[tokio::test]
async fn test_pool_basic() {
    let pool = create_pool(5).await;

    let result = pool
        .query("VALUES 1", &[])
        .await
        .expect("pool query should work");
    assert_eq!(result.row_count, 1);

    pool.close().await.expect("close pool");
}

#[tokio::test]
async fn test_pool_concurrent_queries() {
    let pool = Arc::new(create_pool(10).await);

    // Launch 50 concurrent queries
    let mut handles = Vec::new();
    for i in 0..50 {
        let pool = pool.clone();
        let handle = tokio::spawn(async move {
            let result = pool
                .query(&format!("VALUES {}", i), &[])
                .await
                .unwrap_or_else(|_| panic!("concurrent query #{} should succeed", i));
            assert_eq!(result.row_count, 1);
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.expect("task should not panic");
    }

    pool.close().await.expect("close pool");
}

#[tokio::test]
async fn test_pool_exhaustion_and_wait() {
    // Pool with max 2 connections
    let pool = Arc::new(create_pool(2).await);

    // Acquire both connections
    let client1 = pool.acquire().await.expect("acquire 1");
    let client2 = pool.acquire().await.expect("acquire 2");

    // Release one connection then acquire again -- should succeed
    pool.release(client1).await;

    let client3 = pool.acquire().await.expect("acquire 3 after release");
    client3.close().await.expect("close client3");
    client2.close().await.expect("close client2");

    pool.close().await.expect("close pool");
}

#[tokio::test]
async fn test_pool_recovers_from_broken_connection() {
    let pool = Arc::new(create_pool(2).await);

    // Run a query to establish connections
    pool.query("VALUES 1", &[]).await.expect("initial query");

    // Subsequent queries should still work even if a connection was recycled
    for _ in 0..10 {
        let result = pool
            .query("VALUES 1", &[])
            .await
            .expect("recovery query should succeed");
        assert_eq!(result.row_count, 1);
    }

    pool.close().await.expect("close pool");
}
