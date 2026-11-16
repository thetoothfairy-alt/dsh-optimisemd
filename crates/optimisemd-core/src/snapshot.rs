use crate::store::{Budgets, Entry};

/// A session sees: `user-global` everywhere (the owner's preferences), and the agent layer of its
/// own preset plus the shared one. A session WITHOUT a preset (empty key) sees the shared layer
/// only — before v0.4.8 it saw every preset's private entries, which was a leak, not a contract.
pub fn visible_entries<'a>(
    entries: &'a [Entry],
    workspace_key: &str,
    agent_key: &str,
) -> Vec<&'a Entry> {
    entries
        .iter()
        .filter(|e| {
            let scope_ok = e.scope == "user-global"
                || (e.scope == "workspace" && e.workspace_key == workspace_key);
            let layer_ok = e.track == "user"
                || e.agent_key.is_empty()
                || (!agent_key.is_empty() && e.agent_key == agent_key);
            scope_ok && layer_ok
        })
        .collect()
}

/// The frozen block injected once per session (systemPrompt section, order -50 by default).
/// Kept terse on purpose: it is read by a model, not a person.
pub fn render_snapshot(
    entries: &[Entry],
    budgets: &Budgets,
    workspace_key: &str,
    agent_key: &str,
) -> String {
    let visible = visible_entries(entries, workspace_key, agent_key);
    // The two canonical pairs first, then any pair the store actually holds: before v0.4.8 the
    // other two combinations were stored, spent budget, and rendered for no one.
    let mut groups: Vec<(&str, &str, Vec<&Entry>)> = vec![
        ("user", "user-global", vec![]),
        ("agent", "workspace", vec![]),
    ];
    for e in &visible {
        match groups
            .iter_mut()
            .find(|(t, s, _)| *t == e.track && *s == e.scope)
        {
            Some(g) => g.2.push(e),
            None => groups.push((e.track.as_str(), e.scope.as_str(), vec![e])),
        }
    }
    for g in &mut groups {
        g.2.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
    }
    let used = |track: &str, scope: &str| -> usize {
        visible
            .iter()
            .filter(|e| e.track == track && e.scope == scope)
            .map(|e| e.text.chars().count())
            .sum()
    };
    let mut out = String::new();
    let mut heads: Vec<String> = Vec::new();
    for (track, scope, _) in &groups {
        let cap = if *track == "user" {
            budgets.user
        } else {
            budgets.agent
        };
        heads.push(format!("{track}/{scope} {}/{}", used(track, scope), cap));
    }
    out.push_str(&format!("Tier-0 memory — {}\n", heads.join(" · ")));
    let mut wrote_any = false;
    for (track, scope, list) in &groups {
        if list.is_empty() {
            continue;
        }
        wrote_any = true;
        out.push_str(&format!("\n[{track}/{scope}]\n"));
        for e in list {
            out.push_str("- ");
            out.push_str(e.text.trim());
            out.push('\n');
        }
    }
    if !wrote_any {
        out.push_str("\n(empty; the repo carries the beliefs)\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, track: &str, scope: &str, text: &str, ws: &str, agent: &str) -> Entry {
        Entry {
            id: id.into(),
            track: track.into(),
            scope: scope.into(),
            workspace_key: ws.into(),
            agent_key: agent.into(),
            text: text.into(),
            tags: vec![],
            source: "test".into(),
            created_at: 1,
        }
    }

    #[test]
    fn visibility_is_workspace_and_agent_scoped() {
        let entries = vec![
            entry("1", "user", "user-global", "pref", "ws", ""),
            entry("2", "agent", "workspace", "here", "ws", ""),
            entry("3", "agent", "workspace", "elsewhere", "other", ""),
            entry("4", "agent", "workspace", "sameagent", "ws", "preset-a"),
        ];
        let visible = visible_entries(&entries, "ws", "preset-b");
        let ids: Vec<&str> = visible.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["1", "2"]);
    }

    #[test]
    fn an_empty_agent_key_sees_only_the_shared_layer() {
        // Changed 2026-11-16 (v0.4.8): the old rule gave a session without a preset EVERY preset's
        // entries — a leak, not a contract. A missing preset now sees the shared layer only; a named
        // preset sees shared plus its own.
        let entries = vec![
            entry("1", "user", "user-global", "pref", "ws", "standard"),
            entry("2", "agent", "workspace", "shared here", "ws", ""),
            entry(
                "3",
                "agent",
                "workspace",
                "preset private",
                "ws",
                "standard",
            ),
        ];
        let visible = visible_entries(&entries, "ws", "");
        let ids: Vec<&str> = visible.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["1", "2"]);
    }

    #[test]
    fn snapshot_has_a_budget_header_and_groups() {
        let entries = vec![
            entry("1", "user", "user-global", "keep it plain", "ws", ""),
            entry("2", "agent", "workspace", "the pointer", "ws", ""),
        ];
        let text = render_snapshot(&entries, &Budgets::default(), "ws", "");
        assert!(text.contains("user/user-global 13/2000"), "{text}");
        assert!(
            text.contains("[user/user-global]\n- keep it plain"),
            "{text}"
        );
        assert!(text.contains("[agent/workspace]\n- the pointer"), "{text}");
        assert!(text.contains("agent/workspace 11/4000"), "{text}");
    }

    #[test]
    fn empty_store_says_so() {
        let text = render_snapshot(&[], &Budgets::default(), "ws", "");
        assert!(text.contains("(empty"), "{text}");
    }

    #[test]
    fn every_stored_pair_renders_in_the_snapshot() {
        // user/workspace and agent/user-global are accepted by the store; the snapshot must render them,
        // not silently drop what it shows no one.
        let entries = vec![
            entry(
                "1",
                "user",
                "workspace",
                "a per-workspace preference",
                "ws",
                "",
            ),
            entry("2", "agent", "user-global", "a shared agent note", "ws", ""),
        ];
        let text = render_snapshot(&entries, &Budgets::default(), "ws", "");
        assert!(
            text.contains("[user/workspace]\n- a per-workspace preference"),
            "{text}"
        );
        assert!(
            text.contains("[agent/user-global]\n- a shared agent note"),
            "{text}"
        );
    }

    #[test]
    fn a_session_without_a_preset_sees_only_the_shared_layer() {
        // A missing preset must not open every preset's private entries to the session.
        let entries = vec![
            entry("1", "agent", "workspace", "shared here", "ws", ""),
            entry(
                "2",
                "agent",
                "workspace",
                "preset-private",
                "ws",
                "standard",
            ),
        ];
        let visible = visible_entries(&entries, "ws", "");
        let ids: Vec<&str> = visible.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["1"]);
    }
}
