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
    assert_eq!(snapshot.events[0].event_id, format!("{id}:1"));
    assert_eq!(snapshot.events[1].event_id, format!("{id}:2"));
    drop(snapshot);
    tokio::fs::remove_dir_all(dir).await.unwrap();
}

#[tokio::test]
async fn old_events_receive_stable_ids_without_duplicate_replay() {
    let dir = std::env::temp_dir().join(format!("oldif-store-test-{}", uuid::Uuid::new_v4()));
    let store = Store::open(dir.clone()).await.unwrap();
    let (run, _) = store.create(request()).await.unwrap();
    store.finish(&run, "needs_review", json!({})).await.unwrap();
    let id = run.snapshot.lock().await.run_id.clone();
    let path = dir.join(format!("{id}.json"));
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
    legacy["events"][0]
        .as_object_mut()
        .unwrap()
        .remove("event_id");
    tokio::fs::write(&path, serde_json::to_vec(&legacy).unwrap())
        .await
        .unwrap();
    for _ in 0..2 {
        let reopened = Store::open(dir.clone())
            .await
            .unwrap()
            .get(&id)
            .await
            .unwrap();
        let snapshot = reopened.snapshot.lock().await;
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.events[0].event_id, format!("{id}:1"));
    }
    tokio::fs::remove_dir_all(dir).await.unwrap();
}

#[test]
fn request_boundary_rejects_unknown_run_and_source_fields() {
    let mut value = json!(request());
    value["unknown_permission"] = json!(true);
    assert!(serde_json::from_value::<RunRequest>(value).is_err());
    let source = json!({"id":"s","title":"s","text":"s","url":"https://example.org","page":null,"sha256":"a".repeat(64),"licence":"CC0","human_review_override":true});
    assert!(serde_json::from_value::<crate::api::dto::Source>(source).is_err());
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
