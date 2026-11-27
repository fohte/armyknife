use std::collections::HashSet;
use std::io;
use std::sync::Arc;

use chrono::{DateTime, Utc};

use super::super::state::ClaudeParserState;
use super::super::transcript_branch::active_record_ids;
use super::super::{ParsedTranscript, ParserAdditions};
use super::ingest::{RecordOrigin, ingest_record};
use super::{IncrementalClaudeParser, parse_record};

#[derive(Clone)]
pub(super) struct ActiveRecordCache {
    pub(super) latest_record_uuid: Option<String>,
    pub(super) active_record_ids: Arc<HashSet<usize>>,
    active_branch_path_ids: HashSet<usize>,
}

impl IncrementalClaudeParser {
    pub(in crate::commands::agent::watch_web::messages) fn render_with(
        &self,
        now: DateTime<Utc>,
        additions: ParserAdditions<'_>,
    ) -> io::Result<ParsedTranscript> {
        let pending = self.lines.pending();
        if let Some(record) = parse_record(pending) {
            let mut state = self.state.clone();
            if !ingest_record(
                &mut state,
                &mut self.append_history.clone(),
                record,
                now,
                RecordOrigin::Pending {
                    is_initial: self.pending_is_initial,
                },
                None,
            ) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Claude transcript tail forks from the cached active branch",
                ));
            }
            let active_record_ids = (self.tail_window_start_offset.is_none()
                && state.has_transcript_forks)
                .then(|| {
                    Arc::new(active_record_ids(
                        &state.transcript_nodes,
                        state.latest_record_uuid.as_deref(),
                    ))
                });
            return Ok(super::super::render::render(
                &state,
                self.image_directory.as_deref(),
                now,
                additions,
                active_record_ids.as_deref(),
            ));
        }
        let active_record_ids = (self.tail_window_start_offset.is_none()
            && self.state.has_transcript_forks)
            .then(|| self.cached_active_record_ids(&self.state));
        Ok(super::super::render::render(
            &self.state,
            self.image_directory.as_deref(),
            now,
            additions,
            active_record_ids.as_deref(),
        ))
    }

    pub(super) fn cached_active_record_ids(
        &self,
        state: &ClaudeParserState,
    ) -> Arc<HashSet<usize>> {
        let latest_record_uuid = state.latest_record_uuid.clone();
        let cached = self
            .active_record_cache
            .borrow()
            .as_ref()
            .filter(|cache| cache.latest_record_uuid == latest_record_uuid)
            .map(|cache| Arc::clone(&cache.active_record_ids));
        if let Some(cached) = cached {
            return cached;
        }

        let active_record_ids = Arc::new(active_record_ids(
            &state.transcript_nodes,
            latest_record_uuid.as_deref(),
        ));
        *self.active_record_cache.borrow_mut() = Some(ActiveRecordCache {
            latest_record_uuid,
            active_record_ids: Arc::clone(&active_record_ids),
            active_branch_path_ids: active_branch_path_ids(
                state,
                state.latest_record_uuid.as_deref(),
            ),
        });
        active_record_ids
    }

    pub(super) fn extend_active_record_cache(&mut self, added_ids: Vec<usize>) {
        let Some(cache) = self.active_record_cache.get_mut().as_mut() else {
            return;
        };
        let latest_record_uuid = self.state.latest_record_uuid.clone();
        if cache.latest_record_uuid != latest_record_uuid {
            let mut common_ancestor_id = None;
            let mut current_uuid = latest_record_uuid.as_deref();
            while let Some(uuid) = current_uuid {
                let Some(node) = self.state.transcript_nodes.get(uuid) else {
                    break;
                };
                if cache.active_branch_path_ids.contains(&node.uuid_id) {
                    common_ancestor_id = Some(node.uuid_id);
                    break;
                }
                cache.active_branch_path_ids.insert(node.uuid_id);
                Arc::make_mut(&mut cache.active_record_ids).insert(node.uuid_id);
                current_uuid = node.parent_uuid.as_deref();
            }

            if cache.latest_record_uuid.is_some() && common_ancestor_id.is_none() {
                cache.active_record_ids = Arc::new(active_record_ids(
                    &self.state.transcript_nodes,
                    latest_record_uuid.as_deref(),
                ));
                cache.active_branch_path_ids =
                    active_branch_path_ids(&self.state, latest_record_uuid.as_deref());
            } else {
                let mut old_uuid = cache.latest_record_uuid.as_deref();
                while let Some(uuid) = old_uuid {
                    let Some(node) = self.state.transcript_nodes.get(uuid) else {
                        break;
                    };
                    if Some(node.uuid_id) == common_ancestor_id {
                        break;
                    }
                    cache.active_branch_path_ids.remove(&node.uuid_id);
                    if !node.is_tool_result_only && !node.is_task_notification {
                        Arc::make_mut(&mut cache.active_record_ids).remove(&node.uuid_id);
                    }
                    old_uuid = node.parent_uuid.as_deref();
                }
            }
        }
        for uuid_id in added_ids {
            Arc::make_mut(&mut cache.active_record_ids).insert(uuid_id);
        }
        cache.latest_record_uuid = latest_record_uuid;
    }
}

fn active_branch_path_ids(
    state: &ClaudeParserState,
    latest_record_uuid: Option<&str>,
) -> HashSet<usize> {
    let mut active_branch_path_ids = HashSet::new();
    let mut current_uuid = latest_record_uuid;
    while let Some(uuid) = current_uuid {
        let Some(node) = state.transcript_nodes.get(uuid) else {
            break;
        };
        if !active_branch_path_ids.insert(node.uuid_id) {
            break;
        }
        current_uuid = node.parent_uuid.as_deref();
    }
    active_branch_path_ids
}
