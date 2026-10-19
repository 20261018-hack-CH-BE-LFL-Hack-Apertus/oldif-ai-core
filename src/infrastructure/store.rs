use crate::{
    api::dto::{Event, RunRequest, RunSnapshot},
    domain,
    error::CoreError,
};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, Notify};
pub struct Run {
    pub snapshot: Mutex<RunSnapshot>,
    pub notify: Notify,
    pub cancelled: std::sync::atomic::AtomicBool,
    pub steering: Mutex<Vec<String>>,
}
#[derive(Clone)]
pub struct Store {
    pub runs: Arc<Mutex<HashMap<String, Arc<Run>>>>,
    pub dir: std::path::PathBuf,
}
impl Store {
    pub async fn open(dir: std::path::PathBuf) -> Result<Self, CoreError> {
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|_| CoreError::Storage)?;
        let store = Self {
            runs: Arc::new(Mutex::new(HashMap::new())),
            dir,
        };
        let mut entries = tokio::fs::read_dir(&store.dir)
            .await
            .map_err(|_| CoreError::Storage)?;
        while let Some(e) = entries.next_entry().await.map_err(|_| CoreError::Storage)? {
            if e.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = tokio::fs::read(e.path())
                .await
                .map_err(|_| CoreError::Storage)?;
            let mut snapshot: RunSnapshot =
                serde_json::from_slice(&bytes).map_err(|_| CoreError::Storage)?;
            let mut migrated = false;
            for (index, event) in snapshot.events.iter_mut().enumerate() {
                if event.run_id != snapshot.run_id || event.sequence != index as u64 + 1 {
                    return Err(CoreError::Storage);
                }
                let expected = format!("{}:{}", event.run_id, event.sequence);
                if event.event_id.is_empty() {
                    event.event_id = expected;
                    migrated = true;
                } else if event.event_id != expected {
                    return Err(CoreError::Storage);
                }
            }
            if !domain::terminal(&snapshot.status) {
                snapshot.status = "failed".into();
                snapshot.result = Some(
                    json!({"error":"Service restarted; interrupted runs are never automatically rebilled"}),
                );
            }
            if snapshot
                .events
                .last()
                .is_none_or(|e| e.kind != "run.completed")
                && domain::terminal(&snapshot.status)
            {
                snapshot.events.push(Event {
                    event_id: format!("{}:{}", snapshot.run_id, snapshot.events.len() + 1),
                    run_id: snapshot.run_id.clone(),
                    sequence: snapshot.events.len() as u64 + 1,
                    kind: "run.completed".into(),
                    agent: "outcome-owner".into(),
                    data: json!({"status":snapshot.status,"result":snapshot.result}),
                    created_at: chrono::Utc::now().to_rfc3339(),
                });
                store.persist(&snapshot).await?;
            } else if migrated {
                store.persist(&snapshot).await?;
            }
            store
                .runs
                .lock()
                .await
                .insert(snapshot.run_id.clone(), Arc::new(Run::new(snapshot)));
        }
        Ok(store)
    }
    pub async fn create(&self, request: RunRequest) -> Result<(Arc<Run>, bool), CoreError> {
        let mut runs = self.runs.lock().await;
        if let Some(key) = &request.idempotency_key {
            for r in runs.values() {
                let s = r.snapshot.lock().await;
                if s.request.idempotency_key.as_ref() == Some(key)
                    && s.request.user_id == request.user_id
                {
                    if serde_json::to_value(&s.request).ok() != serde_json::to_value(&request).ok()
                    {
                        return Err(CoreError::Invalid(
                            "Idempotency key was already used for different input".into(),
                        ));
                    }
                    return Ok((r.clone(), false));
                }
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        let run = Arc::new(Run::new(RunSnapshot {
            run_id: id.clone(),
            status: "queued".into(),
            request,
            events: vec![],
            result: None,
        }));
        self.persist(&*run.snapshot.lock().await).await?;
        runs.insert(id, run.clone());
        Ok((run, true))
    }
    pub async fn get(&self, id: &str) -> Result<Arc<Run>, CoreError> {
        self.runs
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or(CoreError::NotFound)
    }
    pub async fn emit(
        &self,
        run: &Run,
        kind: &str,
        agent: &str,
        data: Value,
    ) -> Result<(), CoreError> {
        let mut s = run.snapshot.lock().await;
        let event = Event {
            event_id: format!("{}:{}", s.run_id, s.events.len() + 1),
            run_id: s.run_id.clone(),
            sequence: s.events.len() as u64 + 1,
            kind: kind.into(),
            agent: agent.into(),
            data,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        s.events.push(event);
        self.persist(&s).await?;
        run.notify.notify_waiters();
        Ok(())
    }
    pub async fn finish(&self, run: &Run, status: &str, result: Value) -> Result<(), CoreError> {
        let mut snapshot = run.snapshot.lock().await;
        snapshot.status = status.into();
        snapshot.result = Some(result.clone());
        let event = Event {
            event_id: format!("{}:{}", snapshot.run_id, snapshot.events.len() + 1),
            run_id: snapshot.run_id.clone(),
            sequence: snapshot.events.len() as u64 + 1,
            kind: "run.completed".into(),
            agent: "outcome-owner".into(),
            data: json!({"status":status,"result":result}),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        snapshot.events.push(event);
        self.persist(&snapshot).await?;
        run.notify.notify_waiters();
        Ok(())
    }
    pub async fn persist(&self, snapshot: &RunSnapshot) -> Result<(), CoreError> {
        let path = self.dir.join(format!("{}.json", snapshot.run_id));
        let tmp = path.with_extension("tmp");
        tokio::fs::write(
            &tmp,
            serde_json::to_vec(snapshot).map_err(|_| CoreError::Storage)?,
        )
        .await
        .map_err(|_| CoreError::Storage)?;
        tokio::fs::rename(tmp, path)
            .await
            .map_err(|_| CoreError::Storage)
    }
}
impl Run {
    fn new(snapshot: RunSnapshot) -> Self {
        Self {
            snapshot: Mutex::new(snapshot),
            notify: Notify::new(),
            cancelled: std::sync::atomic::AtomicBool::new(false),
            steering: Mutex::new(vec![]),
        }
    }
}
