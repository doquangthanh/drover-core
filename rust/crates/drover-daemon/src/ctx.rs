//! Everything an RPC handler may need, in one object. Port of `daemon_context.py`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use serde_json::{Value, json};
use tokio::sync::Notify;

use drover_core::status_tracker::now_secs;

use crate::event_hub::EventHub;
use crate::manager::SharedManager;
use crate::settings::Settings;
use crate::state_store::StateStore;

pub struct Ctx {
    pub settings: Arc<Mutex<Settings>>,
    pub hub: Arc<EventHub>,
    pub chats: Arc<crate::chat::manager::ChatManager>,
    pub remotes: Arc<crate::remote::hub::RemoteHub>,
    pub manager: SharedManager,
    pub store: StateStore,
    pub rule_errors: Mutex<Vec<String>>,
    pub token: String,
    pub port: AtomicU16,
    pub started_at: f64,
    pub clients: Mutex<IndexMap<u64, Value>>,
    /// Agent answers to permission prompts waiting for the user: id -> (target pane, data, caller).
    pub pending_answers: Mutex<IndexMap<String, (String, String, String)>>,
    pub stop: Notify,
    pub stopping: AtomicBool,
    /// Start a fresh daemon once this one has shut down (self-update).
    pub restart: AtomicBool,
    /// Per pane: keeps one prompt's paste and Enter together / serialises viewport-scrolling reads.
    pub input_locks: PaneLocks,
    pub read_locks: PaneLocks,
    /// Latest post-run check result per folder.
    pub checks: Arc<crate::checks::CheckStore>,
}

#[derive(Default)]
pub struct PaneLocks(Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>);

impl PaneLocks {
    pub fn get(&self, pane_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.0.lock().unwrap().entry(pane_id.to_string()).or_default().clone()
    }
}

impl Ctx {
    pub fn request_stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.stop.notify_waiters();
        self.stop.notify_one();
    }

    pub fn input_lock(&self, pane_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.input_locks.get(pane_id)
    }

    pub fn read_lock(&self, pane_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.read_locks.get(pane_id)
    }

    pub fn port(&self) -> u16 {
        self.port.load(Ordering::SeqCst)
    }

    pub fn save_now(&self) {
        let data = crate::persistence::serialize(&self.manager.lock().unwrap());
        self.store.save(&data);
    }

    /// Local snapshot plus remote machines, chats and the settings the UI needs.
    pub fn combined_state(&self) -> Value {
        let mut state = self.manager.lock().unwrap().snapshot();
        let s = self.settings.lock().unwrap();
        let obj = state.as_object_mut().expect("snapshot is an object");
        obj.insert("machines".into(), self.remotes.public_state());
        obj.insert("chats".into(), self.chats.snapshot());
        obj.insert("prefix".into(), json!(s.str("prefix", "C-a")));
        obj.insert("tui_prefix".into(), json!(s.str("tui_prefix", "C-b")));
        obj.insert("theme".into(), json!(s.str("theme", "dark")));
        obj.insert("file_icons".into(), json!(s.str("file_icons", "material")));
        obj.insert("terminal_font".into(), crate::fonts::default_terminal_font());
        obj.insert("notify".into(), s.get("notify").cloned().unwrap_or(json!({})));
        obj.insert("orchestration".into(), s.get("orchestration").cloned().unwrap_or(json!({})));
        let pending: Vec<Value> = self.pending_answers.lock().unwrap().iter()
            .map(|(k, (target, text, source))| json!({"id": k, "target": target, "text": text.trim(), "source": source}))
            .collect();
        obj.insert("checks".into(), self.checks.snapshot());
        obj.insert("policy_pending".into(), json!(pending));
        state
    }

    pub fn uptime(&self) -> f64 {
        now_secs() - self.started_at
    }
}
