use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};

use crate::commands::cc::store::SORT_STABILITY_THRESHOLD_SECS;
use crate::commands::cc::types::Session;

/// Tracks the connector type inherited from each ancestor level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TreePrefixSegment {
    /// "│   " - ancestor at this depth is NOT the last child
    Pipe,
    /// "    " - ancestor at this depth IS the last child
    Space,
}

/// A tree node wrapping a session with tree layout metadata.
#[derive(Debug)]
pub(super) struct TreeEntry<'a> {
    pub session: &'a Session,
    /// Depth in the tree (0 = root)
    pub depth: usize,
    /// Whether this node is the last child of its parent
    pub is_last_child: bool,
    /// Prefix segments inherited from ancestors (length = depth)
    pub prefix_segments: Vec<TreePrefixSegment>,
    /// Whether this node has any children
    pub has_children: bool,
}

/// Builds a tree structure from a flat list of sessions.
///
/// Sessions are organized into trees using `ancestor_session_ids`.
/// Each session finds its nearest living ancestor among the displayed sessions.
/// Sessions without parents (or whose parents are not in the list) become roots.
pub(super) fn build_session_tree<'a>(sessions: &[&'a Session]) -> Vec<TreeEntry<'a>> {
    if sessions.is_empty() {
        return Vec::new();
    }

    // Build a set of session IDs that are currently displayed
    let displayed_ids: HashSet<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();

    // For each session, find its parent (nearest living ancestor) among displayed sessions
    let mut parent_map: HashMap<&str, &str> = HashMap::new();
    for session in sessions {
        if let Some(parent_id) = find_nearest_living_ancestor(session, &displayed_ids) {
            parent_map.insert(session.session_id.as_str(), parent_id);
        }
    }

    // Build children map: parent_id -> Vec<child session>
    let mut children_map: HashMap<&str, Vec<&Session>> = HashMap::new();
    let mut root_sessions: Vec<&Session> = Vec::new();

    for &session in sessions {
        if let Some(&parent_id) = parent_map.get(session.session_id.as_str()) {
            children_map.entry(parent_id).or_default().push(session);
        } else {
            root_sessions.push(session);
        }
    }

    // Compute effective_updated_at for each session (subtree maximum)
    let mut effective_map: HashMap<&str, DateTime<Utc>> = HashMap::new();
    for &session in sessions {
        compute_effective_updated_at(
            session.session_id.as_str(),
            session.updated_at,
            &children_map,
            &mut effective_map,
        );
    }

    // Sort root sessions and children by effective_updated_at
    sort_siblings_by_effective_time(&mut root_sessions, &effective_map);
    for children in children_map.values_mut() {
        sort_siblings_by_effective_time(children, &effective_map);
    }

    // Build set of sessions that have children
    let has_children: HashSet<&str> = children_map.keys().copied().collect();

    // DFS to flatten tree into ordered entries
    let mut entries = Vec::new();

    for (root_idx, root_session) in root_sessions.iter().enumerate() {
        let is_last_root = root_idx == root_sessions.len() - 1;
        build_tree_entries_dfs(
            root_session,
            0,
            is_last_root,
            &[],
            &children_map,
            &has_children,
            &mut entries,
        );
    }

    entries
}

/// Recursively builds tree entries via depth-first traversal.
fn build_tree_entries_dfs<'a>(
    session: &'a Session,
    depth: usize,
    is_last_child: bool,
    parent_prefix: &[TreePrefixSegment],
    children_map: &HashMap<&str, Vec<&'a Session>>,
    has_children_set: &HashSet<&str>,
    entries: &mut Vec<TreeEntry<'a>>,
) {
    let has_children = has_children_set.contains(session.session_id.as_str());

    entries.push(TreeEntry {
        session,
        depth,
        is_last_child,
        prefix_segments: parent_prefix.to_vec(),
        has_children,
    });

    if let Some(children) = children_map.get(session.session_id.as_str()) {
        // Build new prefix for children: append segment for current node
        let mut child_prefix = parent_prefix.to_vec();
        if depth > 0 {
            // Non-root nodes contribute a connector to their children's prefix
            if is_last_child {
                child_prefix.push(TreePrefixSegment::Space);
            } else {
                child_prefix.push(TreePrefixSegment::Pipe);
            }
        }

        for (i, child) in children.iter().enumerate() {
            let is_last = i == children.len() - 1;
            build_tree_entries_dfs(
                child,
                depth + 1,
                is_last,
                &child_prefix,
                children_map,
                has_children_set,
                entries,
            );
        }
    }
}

/// Finds the nearest living ancestor of a session among the displayed sessions.
/// Walks `ancestor_session_ids` from the end (nearest ancestor) to the start (root).
fn find_nearest_living_ancestor<'a>(
    session: &'a Session,
    displayed_ids: &HashSet<&str>,
) -> Option<&'a str> {
    // Walk from nearest ancestor to root
    for ancestor_id in session.ancestor_session_ids.iter().rev() {
        if displayed_ids.contains(ancestor_id.as_str()) {
            return Some(ancestor_id.as_str());
        }
    }
    None
}

/// Computes the effective updated_at for a session by taking the maximum
/// of its own updated_at and all descendants' updated_at (recursively).
/// Results are memoized in the cache.
fn compute_effective_updated_at<'a>(
    session_id: &'a str,
    updated_at: DateTime<Utc>,
    children_map: &HashMap<&str, Vec<&'a Session>>,
    cache: &mut HashMap<&'a str, DateTime<Utc>>,
) -> DateTime<Utc> {
    if let Some(&cached) = cache.get(session_id) {
        return cached;
    }

    let mut max_time = updated_at;

    if let Some(children) = children_map.get(session_id) {
        for child in children {
            let child_effective = compute_effective_updated_at(
                child.session_id.as_str(),
                child.updated_at,
                children_map,
                cache,
            );
            if child_effective > max_time {
                max_time = child_effective;
            }
        }
    }

    cache.insert(session_id, max_time);
    max_time
}

/// Sorts siblings by their effective updated_at (subtree maximum) descending,
/// using the same bucket-based stability threshold as the flat sort.
/// Final tiebreaker by session_id for deterministic ordering.
fn sort_siblings_by_effective_time(
    siblings: &mut Vec<&Session>,
    effective_map: &HashMap<&str, DateTime<Utc>>,
) {
    siblings.sort_by(|a, b| {
        let eff_a = effective_map
            .get(a.session_id.as_str())
            .copied()
            .unwrap_or(a.updated_at);
        let eff_b = effective_map
            .get(b.session_id.as_str())
            .copied()
            .unwrap_or(b.updated_at);

        let bucket_a = eff_a.timestamp() / SORT_STABILITY_THRESHOLD_SECS;
        let bucket_b = eff_b.timestamp() / SORT_STABILITY_THRESHOLD_SECS;

        bucket_b
            .cmp(&bucket_a)
            .then_with(|| b.created_at.cmp(&a.created_at))
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
}

/// Builds the tree connector prefix string for the first line of a node.
///
/// For root nodes (depth=0): no prefix (empty string)
/// For children: inherited prefix + own connector ("├── " or "└── ")
pub(super) fn build_line1_tree_prefix(entry: &TreeEntry) -> String {
    if entry.depth == 0 {
        return String::new();
    }

    let mut prefix = String::new();
    for segment in &entry.prefix_segments {
        match segment {
            TreePrefixSegment::Pipe => prefix.push_str("│   "),
            TreePrefixSegment::Space => prefix.push_str("    "),
        }
    }

    if entry.is_last_child {
        prefix.push_str("└── ");
    } else {
        prefix.push_str("├── ");
    }

    prefix
}

/// Builds the tree connector prefix string for the second line (continuation).
///
/// For root nodes: no prefix
/// For children: inherited prefix + continuation ("│   " if not last, "    " if last)
pub(super) fn build_line2_tree_prefix(entry: &TreeEntry) -> String {
    if entry.depth == 0 {
        return String::new();
    }

    let mut prefix = String::new();
    for segment in &entry.prefix_segments {
        match segment {
            TreePrefixSegment::Pipe => prefix.push_str("│   "),
            TreePrefixSegment::Space => prefix.push_str("    "),
        }
    }

    if entry.is_last_child {
        prefix.push_str("    ");
    } else {
        prefix.push_str("│   ");
    }

    prefix
}

/// Builds the tree connector prefix for separator lines between siblings.
///
/// For root-level separators: "│" below parent
/// For child-level separators: inherited prefix + "│"
pub(super) fn build_separator_tree_prefix(entry: &TreeEntry) -> String {
    if entry.depth == 0 {
        // Root sessions that have children: show "│" below them
        if entry.has_children {
            return "│".to_string();
        }
        return String::new();
    }

    let mut prefix = String::new();
    for segment in &entry.prefix_segments {
        match segment {
            TreePrefixSegment::Pipe => prefix.push_str("│   "),
            TreePrefixSegment::Space => prefix.push_str("    "),
        }
    }

    // After the last child's separator, only show pipe if not last
    if !entry.is_last_child {
        prefix.push('│');
    }

    prefix
}

/// Builds the tree connector prefix for lines between a parent and its children.
///
/// Shows "│" at the parent's depth level to connect parent to children block.
pub(super) fn build_parent_child_connector(entry: &TreeEntry) -> String {
    if entry.depth == 0 {
        return "│".to_string();
    }

    let mut prefix = String::new();
    for segment in &entry.prefix_segments {
        match segment {
            TreePrefixSegment::Pipe => prefix.push_str("│   "),
            TreePrefixSegment::Space => prefix.push_str("    "),
        }
    }

    // Continue the pipe from parent's connector position
    if entry.is_last_child {
        prefix.push_str("    │");
    } else {
        prefix.push_str("│   │");
    }

    prefix
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::SessionStatus;
    use chrono::{TimeDelta, Utc};
    use rstest::rstest;
    use std::path::PathBuf;

    fn create_test_session(id: &str) -> Session {
        Session {
            session_id: id.to_string(),
            cwd: PathBuf::from("/home/user/project"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            claude_pid: None,
        }
    }

    fn create_test_session_with_times(
        id: &str,
        created_secs_ago: i64,
        updated_secs_ago: i64,
    ) -> Session {
        let now = Utc::now();
        Session {
            session_id: id.to_string(),
            cwd: PathBuf::from("/home/user/project"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: now - TimeDelta::seconds(created_secs_ago),
            updated_at: now - TimeDelta::seconds(updated_secs_ago),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            claude_pid: None,
        }
    }

    // =========================================================================
    // Tree view structure tests
    // =========================================================================

    #[test]
    fn test_build_session_tree_flat_sessions() {
        let s1 = create_test_session("a");
        let s2 = create_test_session("b");
        let sessions: Vec<&Session> = vec![&s1, &s2];

        let tree = build_session_tree(&sessions);

        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].depth, 0);
        assert_eq!(tree[1].depth, 0);
        assert!(!tree[0].has_children);
        assert!(!tree[1].has_children);
    }

    #[test]
    fn test_build_session_tree_parent_child() {
        let parent = create_test_session("parent");
        let mut child = create_test_session("child");
        child.ancestor_session_ids = vec!["parent".to_string()];

        let sessions: Vec<&Session> = vec![&parent, &child];
        let tree = build_session_tree(&sessions);

        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].session.session_id, "parent");
        assert_eq!(tree[0].depth, 0);
        assert!(tree[0].has_children);
        assert_eq!(tree[1].session.session_id, "child");
        assert_eq!(tree[1].depth, 1);
        assert!(tree[1].is_last_child);
    }

    #[test]
    fn test_build_session_tree_skips_deleted_ancestor() {
        // ancestor_session_ids = [root, deleted_middle]
        // Only root is displayed, so child should attach to root
        let root = create_test_session("root");
        let mut child = create_test_session("child");
        child.ancestor_session_ids = vec!["root".to_string(), "deleted_middle".to_string()];

        let sessions: Vec<&Session> = vec![&root, &child];
        let tree = build_session_tree(&sessions);

        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].session.session_id, "root");
        assert!(tree[0].has_children);
        assert_eq!(tree[1].session.session_id, "child");
        assert_eq!(tree[1].depth, 1);
    }

    #[test]
    fn test_build_session_tree_multiple_children() {
        // child1 updated more recently -> sorted first
        let parent = create_test_session_with_times("parent", 120, 120);
        let mut child1 = create_test_session_with_times("child1", 60, 5);
        child1.ancestor_session_ids = vec!["parent".to_string()];
        let mut child2 = create_test_session_with_times("child2", 90, 60);
        child2.ancestor_session_ids = vec!["parent".to_string()];

        let sessions: Vec<&Session> = vec![&parent, &child1, &child2];
        let tree = build_session_tree(&sessions);

        assert_eq!(tree.len(), 3);
        assert_eq!(tree[0].depth, 0);
        assert!(tree[0].has_children);
        assert_eq!(tree[1].session.session_id, "child1");
        assert_eq!(tree[1].depth, 1);
        assert!(!tree[1].is_last_child);
        assert_eq!(tree[2].session.session_id, "child2");
        assert_eq!(tree[2].depth, 1);
        assert!(tree[2].is_last_child);
    }

    #[test]
    fn test_build_session_tree_nested() {
        let root = create_test_session("root");
        let mut mid = create_test_session("mid");
        mid.ancestor_session_ids = vec!["root".to_string()];
        let mut leaf = create_test_session("leaf");
        leaf.ancestor_session_ids = vec!["root".to_string(), "mid".to_string()];

        let sessions: Vec<&Session> = vec![&root, &mid, &leaf];
        let tree = build_session_tree(&sessions);

        assert_eq!(tree.len(), 3);
        assert_eq!(tree[0].depth, 0); // root
        assert_eq!(tree[1].depth, 1); // mid
        assert_eq!(tree[2].depth, 2); // leaf
    }

    // =========================================================================
    // Subtree sort tests
    // =========================================================================

    #[test]
    fn test_roots_sorted_by_child_effective_updated_at() {
        // Root A (updated 2 min ago) has child A1 (updated 5s ago)
        // Root B (updated 30s ago) has no children
        // A's effective = 5s ago, B's effective = 30s ago -> A first
        let root_a = create_test_session_with_times("root_a", 300, 120);
        let mut child_a1 = create_test_session_with_times("child_a1", 60, 5);
        child_a1.ancestor_session_ids = vec!["root_a".to_string()];
        let root_b = create_test_session_with_times("root_b", 200, 35);

        // Pass in "wrong" order to verify sorting corrects it
        let sessions: Vec<&Session> = vec![&root_b, &root_a, &child_a1];
        let tree = build_session_tree(&sessions);

        assert_eq!(tree.len(), 3);
        assert_eq!(tree[0].session.session_id, "root_a");
        assert_eq!(tree[0].depth, 0);
        assert_eq!(tree[1].session.session_id, "child_a1");
        assert_eq!(tree[1].depth, 1);
        assert_eq!(tree[2].session.session_id, "root_b");
        assert_eq!(tree[2].depth, 0);
    }

    #[test]
    fn test_grandchild_propagates_to_root_sort() {
        // Root A (10 min ago) -> child (10 min ago) -> grandchild (5s ago)
        // Root B (5 min ago) -> no children
        // A's effective = 5s ago (from grandchild), B = 5 min ago -> A first
        let root_a = create_test_session_with_times("root_a", 600, 600);
        let mut child = create_test_session_with_times("child", 600, 600);
        child.ancestor_session_ids = vec!["root_a".to_string()];
        let mut grandchild = create_test_session_with_times("grandchild", 60, 5);
        grandchild.ancestor_session_ids = vec!["root_a".to_string(), "child".to_string()];
        let root_b = create_test_session_with_times("root_b", 300, 300);

        let sessions: Vec<&Session> = vec![&root_b, &root_a, &child, &grandchild];
        let tree = build_session_tree(&sessions);

        assert_eq!(tree.len(), 4);
        assert_eq!(tree[0].session.session_id, "root_a");
        assert_eq!(tree[0].depth, 0);
        assert_eq!(tree[1].session.session_id, "child");
        assert_eq!(tree[1].depth, 1);
        assert_eq!(tree[2].session.session_id, "grandchild");
        assert_eq!(tree[2].depth, 2);
        assert_eq!(tree[3].session.session_id, "root_b");
        assert_eq!(tree[3].depth, 0);
    }

    #[test]
    fn test_children_sorted_by_subtree_effective_time() {
        // Parent P has two children:
        //   C1 (updated 2 min ago) has grandchild G1 (updated 5s ago)
        //   C2 (updated 10s ago) has no children
        // C1's effective = 5s ago (from G1), C2's effective = 10s ago
        // Both in same 30s bucket -> tiebreak by created_at
        // Use different buckets to ensure C1 sorts first
        let parent = create_test_session_with_times("parent", 600, 600);
        let mut c1 = create_test_session_with_times("c1", 300, 120);
        c1.ancestor_session_ids = vec!["parent".to_string()];
        let mut g1 = create_test_session_with_times("g1", 60, 5);
        g1.ancestor_session_ids = vec!["parent".to_string(), "c1".to_string()];
        let mut c2 = create_test_session_with_times("c2", 200, 60);
        c2.ancestor_session_ids = vec!["parent".to_string()];

        // Pass c2 before c1 in input to verify sorting
        let sessions: Vec<&Session> = vec![&parent, &c2, &c1, &g1];
        let tree = build_session_tree(&sessions);

        assert_eq!(tree.len(), 4);
        assert_eq!(tree[0].session.session_id, "parent");
        assert_eq!(tree[1].session.session_id, "c1");
        assert_eq!(tree[1].depth, 1);
        assert_eq!(tree[2].session.session_id, "g1");
        assert_eq!(tree[2].depth, 2);
        assert_eq!(tree[3].session.session_id, "c2");
        assert_eq!(tree[3].depth, 1);
    }

    // =========================================================================
    // Tree prefix tests
    // =========================================================================

    #[rstest]
    fn test_tree_prefix_root_node() {
        let session = create_test_session("root");
        let entry = TreeEntry {
            session: &session,
            depth: 0,
            is_last_child: true,
            prefix_segments: vec![],
            has_children: false,
        };

        assert_eq!(build_line1_tree_prefix(&entry), "");
        assert_eq!(build_line2_tree_prefix(&entry), "");
    }

    #[rstest]
    fn test_tree_prefix_first_child() {
        let session = create_test_session("child");
        let entry = TreeEntry {
            session: &session,
            depth: 1,
            is_last_child: false,
            prefix_segments: vec![],
            has_children: false,
        };

        assert_eq!(build_line1_tree_prefix(&entry), "├── ");
        assert_eq!(build_line2_tree_prefix(&entry), "│   ");
    }

    #[rstest]
    fn test_tree_prefix_last_child() {
        let session = create_test_session("child");
        let entry = TreeEntry {
            session: &session,
            depth: 1,
            is_last_child: true,
            prefix_segments: vec![],
            has_children: false,
        };

        assert_eq!(build_line1_tree_prefix(&entry), "└── ");
        assert_eq!(build_line2_tree_prefix(&entry), "    ");
    }

    #[rstest]
    fn test_tree_prefix_nested_with_pipe() {
        let session = create_test_session("deep");
        let entry = TreeEntry {
            session: &session,
            depth: 2,
            is_last_child: true,
            prefix_segments: vec![TreePrefixSegment::Pipe],
            has_children: false,
        };

        assert_eq!(build_line1_tree_prefix(&entry), "│   └── ");
        assert_eq!(build_line2_tree_prefix(&entry), "│       ");
    }
}
