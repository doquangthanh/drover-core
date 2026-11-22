//! Client-side copy of the daemon's state, kept current from /ws events (port of the React store's core).

use std::collections::HashMap;

use serde_json::Value;

use crate::wire::{Node, Pane, Snapshot, Space, Tab};

#[derive(Debug, Default)]
pub struct Store {
    pub spaces: Vec<Space>,
    pub tabs: Vec<Tab>,
    pub panes: Vec<Pane>,
    pub orchestration: Value,
    pub machines: Value,
    pub policy_pending: Vec<Value>,
    pub notify: Value,
    /// Bumped on the daemon's "messages" event (a sub-agent started or stopped): the Agent tree refetches.
    pub messages_tick: u64,
    pane_index: HashMap<String, usize>,
}

/// Most urgent first: what a space or tab shows when its agents differ.
pub fn urgency(state: &str) -> u8 {
    match state {
        "blocked" => 4,
        "done" => 3,
        "working" => 2,
        "idle" => 1,
        _ => 0,
    }
}

/// `local` plus `machines[].snapshot[key]` (port of the React store's `mergeAll`); unreadable entries are skipped.
fn merged<T: serde::de::DeserializeOwned>(mut local: Vec<T>, machines: &Value, key: &str) -> Vec<T> {
    for m in machines.as_array().into_iter().flatten() {
        let remote = m["snapshot"][key].as_array().into_iter().flatten();
        local.extend(remote.filter_map(|v| serde_json::from_value(v.clone()).ok()));
    }
    local
}

impl Store {
    /// Apply one daemon event. Returns true when anything visible changed.
    pub fn apply(&mut self, name: &str, data: &Value) -> bool {
        match name {
            "state" | "resync" => {
                let snap: Snapshot = match serde_json::from_value(data.clone()) {
                    Ok(snap) => snap,
                    Err(e) => {
                        eprintln!("drover-proto: state not understood: {e}");
                        return false;
                    }
                };
                // Local objects first, then every connected machine's (ids already `alias:`-prefixed by the daemon).
                self.spaces = merged(snap.spaces, &snap.machines, "spaces");
                self.tabs = merged(snap.tabs, &snap.machines, "tabs");
                self.panes = merged(snap.panes, &snap.machines, "panes");
                self.orchestration = snap.orchestration;
                self.machines = snap.machines;
                self.policy_pending = snap.policy_pending;
                self.notify = snap.notify;
                self.reindex();
                true
            }
            "pane" => self.upsert(data),
            "status" => self.upsert(&data["pane"]),
            "messages" => {
                self.messages_tick += 1;
                true
            }
            _ => false,
        }
    }

    fn upsert(&mut self, data: &Value) -> bool {
        let Ok(pane) = serde_json::from_value::<Pane>(data.clone()) else { return false };
        match self.pane_index.get(&pane.id) {
            Some(&i) if self.panes[i] == pane => false,
            Some(&i) => {
                self.panes[i] = pane;
                true
            }
            None => {
                self.pane_index.insert(pane.id.clone(), self.panes.len());
                self.panes.push(pane);
                true
            }
        }
    }

    fn reindex(&mut self) {
        self.pane_index = self.panes.iter().enumerate().map(|(i, p)| (p.id.clone(), i)).collect();
    }

    pub fn pane(&self, id: &str) -> Option<&Pane> {
        self.pane_index.get(id).map(|&i| &self.panes[i])
    }

    pub fn tab(&self, id: &str) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn tabs_of<'a>(&'a self, space_id: &'a str) -> impl Iterator<Item = &'a Tab> + 'a {
        self.tabs.iter().filter(move |t| t.space_id == space_id)
    }

    pub fn panes_of_space<'a>(&'a self, space_id: &'a str) -> impl Iterator<Item = &'a Pane> + 'a {
        self.panes.iter().filter(move |p| p.space_id == space_id)
    }

    /// The most urgent state among a space's agents ("" when it has none).
    pub fn space_state(&self, space_id: &str) -> &str {
        self.panes.iter()
            .filter(|p| p.space_id == space_id && p.kind != "shell")
            .map(|p| p.state.as_str())
            .max_by_key(|s| urgency(s))
            .unwrap_or("")
    }

    /// What a tab draws: its layout, or only the focused pane when zoomed.
    pub fn tab_view(&self, tab_id: &str) -> Option<Node> {
        let tab = self.tab(tab_id)?;
        match (&tab.focused, tab.zoomed) {
            (Some(f), true) => Some(Node::Pane { pane: f.clone() }),
            _ => tab.layout.clone(),
        }
    }
}
