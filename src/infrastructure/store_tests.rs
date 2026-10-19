use super::store::Store;
use crate::api::dto::RunRequest;
use serde_json::json;
fn request() -> RunRequest {
    RunRequest {
        question: "Q".into(),
        locale: "en".into(),
        model: "fixture".into(),
        sources: vec![],
        session_id: "s".into(),
        user_id: "u".into(),
        history: vec![],
        idempotency_key: Some("one".into()),
    }
}
#[tokio::test]
async fn idempotency_prevents_duplicate_spend_and_rejects_changed_input() {
    let dir = std::env::temp_dir().join(format!("oldif-store-test-{}", uuid::Uuid::new_v4()));
    let store = Store::open(dir.clone()).await.unwrap();
    let (first, created) = store.create(request()).await.unwrap();
    assert!(created);
    let (second, created) = store.create(request()).await.unwrap();
    assert!(!created);
    assert!(std::sync::Arc::ptr_eq(&first, &second));
    let mut changed = request();
    changed.question = "Different".into();
    assert!(store.create(changed).await.is_err());
    tokio::fs::remove_dir_all(dir).await.unwrap();
}
#[tokio::test]
async fn terminal_status_and_event_persist_atomically() {
    let dir = std::env::temp_dir().join(format!("oldif-store-test-{}", uuid::Uuid::new_v4()));
    let store = Store::open(dir.clone()).await.unwrap();
    let (run, _) = store.create(request()).await.unwrap();
    store
        .emit(&run, "model.started", "researcher", json!({}))
        .await
        .unwrap();
    store
        .finish(&run, "needs_review", json!({"answer":"Q"}))
        .await
        .unwrap();
    let id = run.snapshot.lock().await.run_id.clone();
    let restored = Store::open(dir.clone())
        .await
        .unwrap()
        .get(&id)
        .await
        .unwrap();
    let snapshot = restored.snapshot.lock().await;
    assert_eq!(snapshot.status, "needs_review");
    assert_eq!(snapshot.events.last().unwrap().kind, "run.completed");
    assert_eq!(snapshot.events.last().unwrap().sequence, 2);
    drop(snapshot);
    tokio::fs::remove_dir_all(dir).await.unwrap();
}
#[tokio::test]
async fn restart_marks_interrupted_work_failed_without_retry() {
    let dir = std::env::temp_dir().join(format!("oldif-store-test-{}", uuid::Uuid::new_v4()));
    let store = Store::open(dir.clone()).await.unwrap();
    let (run, _) = store.create(request()).await.unwrap();
    let id = run.snapshot.lock().await.run_id.clone();
    let restored = Store::open(dir.clone())
        .await
        .unwrap()
        .get(&id)
        .await
        .unwrap();
    let snapshot = restored.snapshot.lock().await;
    assert_eq!(snapshot.status, "failed");
    assert_eq!(snapshot.events.last().unwrap().kind, "run.completed");
    drop(snapshot);
    tokio::fs::remove_dir_all(dir).await.unwrap();
}
