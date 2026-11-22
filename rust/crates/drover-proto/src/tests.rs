use serde_json::json;

use crate::store::Store;
use crate::wire::{Node, ServerMsg};

fn snapshot() -> serde_json::Value {
    json!({
        "spaces": [{"id": "s1", "name": "herd", "cwd": "/w/herd", "color": 2, "branch": "main", "dirty": 2, "ahead": 1, "custom_name": false}],
        "tabs": [{"id": "t1", "space_id": "s1", "name": "claude", "focused": "%2", "zoomed": false,
                  "layout": {"split": "row", "ratio": 0.6, "a": {"pane": "%1"}, "b": {"pane": "%2"}}}],
        "panes": [
            {"id": "%1", "name": "claude", "kind": "claude", "space_id": "s1", "tab_id": "t1", "state": "working", "cols": 120, "rows": 40},
            {"id": "%2", "name": "zsh", "kind": "shell", "space_id": "s1", "tab_id": "t1", "state": "blocked", "cols": 80, "rows": 40}
        ],
        "machines": [], "prefix": "C-a"
    })
}

#[test]
fn parses_every_message_kind() {
    assert_eq!(ServerMsg::parse(r#"{"t":"res","id":3,"result":{"ok":1}}"#),
               ServerMsg::Reply { id: 3, result: Ok(json!({"ok": 1})) });
    assert_eq!(ServerMsg::parse(r#"{"t":"res","id":4,"error":{"code":3,"message":"no pane"}}"#),
               ServerMsg::Reply { id: 4, result: Err("no pane".into()) });
    assert_eq!(ServerMsg::parse(r#"{"t":"replay","pane":"%1","data":"hi"}"#),
               ServerMsg::Output { pane: "%1".into(), data: "hi".into(), replay: true });
    assert_eq!(ServerMsg::parse("not json"), ServerMsg::Other);
}

#[test]
fn state_event_fills_the_store_with_git_facts_and_layout() {
    let mut s = Store::default();
    assert!(s.apply("state", &snapshot()));
    assert_eq!(s.spaces[0].branch.as_deref(), Some("main"));
    assert_eq!(s.tabs_of("s1").count(), 1);
    match s.tab_view("t1") {
        Some(Node::Split { ratio, .. }) => assert!((ratio - 0.6).abs() < 1e-9),
        other => panic!("unexpected layout {other:?}"),
    }
    // Shells do not count for a space's state: the claude pane is working, the blocked one is a shell.
    assert_eq!(s.space_state("s1"), "working");
}

#[test]
fn pane_and_status_events_update_one_pane_only_when_it_changed() {
    let mut s = Store::default();
    s.apply("state", &snapshot());
    let mut p = serde_json::to_value(s.pane("%1").cloned().map(|p| json!({"id": p.id, "name": p.name, "kind": p.kind,
        "space_id": p.space_id, "tab_id": p.tab_id, "state": "blocked", "cols": p.cols, "rows": p.rows}))).unwrap();
    assert!(s.apply("status", &json!({"pane": p, "old": "working", "new": "blocked"})));
    assert_eq!(s.space_state("s1"), "blocked");
    assert!(!s.apply("pane", &p), "same data again is not a change");
    p["id"] = json!("%9");
    assert!(s.apply("pane", &p));
    assert!(s.pane("%9").is_some());
}

#[test]
fn zoomed_tab_shows_only_its_focused_pane() {
    let mut snap = snapshot();
    snap["tabs"][0]["zoomed"] = json!(true);
    let mut s = Store::default();
    s.apply("state", &snap);
    assert_eq!(s.tab_view("t1"), Some(Node::Pane { pane: "%2".into() }));
}

#[test]
fn state_event_merges_machine_snapshots_after_local_objects() {
    let mut snap = snapshot();
    snap["machines"] = json!([
        {"alias": "box", "status": "connected", "snapshot": {
            "spaces": [{"id": "box:s1", "name": "api", "machine": "box"}],
            "tabs": [{"id": "box:t1", "space_id": "box:s1", "name": "main"}],
            "panes": [{"id": "box:%1", "name": "claude", "kind": "claude", "space_id": "box:s1", "tab_id": "box:t1", "state": "blocked", "machine": "box"}]}},
        {"alias": "off", "status": "retrying", "snapshot": {}}
    ]);
    let mut s = Store::default();
    s.apply("state", &snap);
    assert_eq!(s.spaces.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["s1", "box:s1"]);
    assert_eq!(s.spaces[1].machine.as_deref(), Some("box"));
    assert!(s.tab("box:t1").is_some());
    assert_eq!(s.pane("box:%1").map(|p| p.machine.as_str()), Some("box"));
    assert_eq!(s.space_state("box:s1"), "blocked");
}

#[test]
fn state_event_keeps_policy_machines_and_orchestration_and_messages_bump_a_tick() {
    let mut s = Store::default();
    let mut snap = snapshot();
    snap["orchestration"] = json!({"max_depth": 3});
    snap["machines"] = json!([{"alias": "box", "connected": true}]);
    snap["policy_pending"] = json!([{"id": "p1", "source": "%1", "target": "%2", "action": "send"}]);
    s.apply("state", &snap);
    assert_eq!(s.orchestration["max_depth"], 3);
    assert_eq!(s.machines[0]["alias"], "box");
    assert_eq!(s.policy_pending.len(), 1);
    assert_eq!(s.messages_tick, 0);
    assert!(s.apply("messages", &json!({"kind": "run"})));
    assert_eq!(s.messages_tick, 1);
}
