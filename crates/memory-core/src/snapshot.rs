use crate::store::{Budgets, Entry};

/// A session sees: shared agent entries plus its own agent key; `user-global` everywhere; `workspace`
/// entries only when the workspace key matches.
pub fn visible_entries<'a>(entries: &'a [Entry], workspace_key: &str, agent_key: &str) -> Vec<&'a Entry> {
    entries.iter()
        .filter(|e| (e.agent_key.is_empty() || e.agent_key == agent_key)
            && (e.scope == "user-global" || (e.scope == "workspace" && e.workspace_key == workspace_key)))
        .collect()
}

/// The frozen block injected once per session (systemPrompt section, order -50 by default).
/// Kept terse on purpose: it is read by a model, not a person.
pub fn render_snapshot(entries: &[Entry], budgets: &Budgets, workspace_key: &str, agent_key: &str) -> String {
    let visible = visible_entries(entries, workspace_key, agent_key);
    let mut groups: Vec<(&str, &str, Vec<&Entry>)> = vec![("user", "user-global", vec![]), ("agent", "workspace", vec![])];
    for e in &visible {
        if let Some(g) = groups.iter_mut().find(|(t, s, _)| *t == e.track && *s == e.scope) {
            g.2.push(e);
        }
    }
    for g in &mut groups {
        g.2.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
    }
    let used = |track: &str, scope: &str| -> usize {
        visible.iter().filter(|e| e.track == track && e.scope == scope).map(|e| e.text.chars().count()).sum()
    };
    let mut out = String::new();
    out.push_str(&format!(
        "Tier-0 memory — user/user-global {}/{} · agent/workspace {}/{}\n",
        used("user", "user-global"), budgets.user, used("agent", "workspace"), budgets.agent,
    ));
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
            id: id.into(), track: track.into(), scope: scope.into(),
            workspace_key: ws.into(), agent_key: agent.into(),
            text: text.into(), tags: vec![], source: "test".into(), created_at: 1,
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
    fn snapshot_has_a_budget_header_and_groups() {
        let entries = vec![
            entry("1", "user", "user-global", "keep it plain", "ws", ""),
            entry("2", "agent", "workspace", "the pointer", "ws", ""),
        ];
        let text = render_snapshot(&entries, &Budgets::default(), "ws", "");
        assert!(text.contains("user/user-global 13/2000"), "{text}");
        assert!(text.contains("[user/user-global]\n- keep it plain"), "{text}");
        assert!(text.contains("[agent/workspace]\n- the pointer"), "{text}");
        assert!(text.contains("agent/workspace 11/4000"), "{text}");
    }

    #[test]
    fn empty_store_says_so() {
        let text = render_snapshot(&[], &Budgets::default(), "ws", "");
        assert!(text.contains("(empty"), "{text}");
    }
}
