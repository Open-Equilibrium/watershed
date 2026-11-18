use std::sync::Arc;

use crate::service::transfers::TransferLocks;

#[tokio::test]
async fn a_cancelled_move_leaves_no_transfer_lock_behind() {
    let locks = Arc::new(TransferLocks::default());
    let (entered, started) = tokio::sync::oneshot::channel();
    let holder = tokio::spawn({
        let locks = locks.clone();
        async move {
            locks
                .run("tr-1", async move {
                    let _ = entered.send(());
                    std::future::pending::<Result<(), String>>().await
                })
                .await
        }
    });
    started.await.unwrap();
    holder.abort();
    let _ = holder.await;
    assert!(locks.is_empty(), "a cancelled move leaves no lock entry");
}
