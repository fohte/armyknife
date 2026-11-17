use std::path::Path;

use anyhow::Result;

use crate::commands::agent::types::{Engine, MAIN_THREAD_AGENT_KEY, Session};
use crate::commands::agent::user_turn::is_ask_user_question;

use super::cache::{MessageCacheKey, MessageCacheRecords};
use super::file_attachments::merge_stored_files;
use super::incremental::{filter_after_tail_window, parse_initial_or_full};
use super::pending_permissions::{collect_pending_permission_cards, permission_denied_cards};
use super::proposals::proposal_chat_item;
use super::questions::attach_question_request_if_present;
use super::routes::RouteLoaders;
use super::session_titles::{parent_session, resolve_peer_session_titles};
use super::subagent_reports;
use super::transcript_path::archived_codex_transcript_path;
use super::{ChatItem, ConversationResponse, TranscriptInput, cache, command_source, parser};

pub(super) fn load_conversation_with_queued_prompts(
    session_id: &str,
    load_session: impl FnMut(&str) -> Result<Option<Session>>,
    load_user_marker: impl FnOnce(&str) -> Result<Option<String>>,
    loaders: &RouteLoaders,
) -> Result<Option<ConversationResponse>> {
    load_conversation_with_tail_window(session_id, load_session, load_user_marker, loaders, None)
}

pub(in crate::commands::agent::watch_web) fn conversation_contains_markdown_source(
    session_id: &str,
    load_session: impl FnMut(&str) -> Result<Option<Session>>,
    load_user_marker: impl FnOnce(&str) -> Result<Option<String>>,
    loaders: &RouteLoaders,
    expected_message: &str,
) -> Result<bool> {
    let Some(conversation) = load_conversation_with_tail_window_and_marker_updates(
        session_id,
        load_session,
        load_user_marker,
        loaders,
        None,
        false,
    )?
    else {
        return Ok(false);
    };

    Ok(command_source::contains_markdown_source(
        &conversation.messages,
        expected_message,
    ))
}

pub(super) fn load_conversation_with_initial_tail_window(
    session_id: &str,
    load_session: impl FnMut(&str) -> Result<Option<Session>>,
    load_user_marker: impl FnOnce(&str) -> Result<Option<String>>,
    loaders: &RouteLoaders,
    message_limit: usize,
) -> Result<Option<ConversationResponse>> {
    load_conversation_with_tail_window(
        session_id,
        load_session,
        load_user_marker,
        loaders,
        Some(message_limit),
    )
}

fn load_conversation_with_tail_window(
    session_id: &str,
    load_session: impl FnMut(&str) -> Result<Option<Session>>,
    load_user_marker: impl FnOnce(&str) -> Result<Option<String>>,
    loaders: &RouteLoaders,
    tail_window_limit: Option<usize>,
) -> Result<Option<ConversationResponse>> {
    load_conversation_with_tail_window_and_marker_updates(
        session_id,
        load_session,
        load_user_marker,
        loaders,
        tail_window_limit,
        true,
    )
}

fn load_conversation_with_tail_window_and_marker_updates(
    session_id: &str,
    load_session: impl FnMut(&str) -> Result<Option<Session>>,
    load_user_marker: impl FnOnce(&str) -> Result<Option<String>>,
    loaders: &RouteLoaders,
    tail_window_limit: Option<usize>,
    mark_resolved_queued_prompts: bool,
) -> Result<Option<ConversationResponse>> {
    let codex_home = crate::shared::dirs::codex_home_dir();
    load_conversation_with_tail_window_from(
        session_id,
        load_session,
        load_user_marker,
        loaders,
        tail_window_limit,
        codex_home.as_deref(),
        mark_resolved_queued_prompts,
    )
}

fn load_conversation_with_tail_window_from(
    session_id: &str,
    mut load_session: impl FnMut(&str) -> Result<Option<Session>>,
    load_user_marker: impl FnOnce(&str) -> Result<Option<String>>,
    loaders: &RouteLoaders,
    tail_window_limit: Option<usize>,
    codex_home: Option<&Path>,
    mark_resolved_queued_prompts: bool,
) -> Result<Option<ConversationResponse>> {
    let Some(mut session) = load_session(session_id)? else {
        return Ok(None);
    };
    if let Some(path) = archived_codex_transcript_path(
        session.engine,
        session.transcript_path.as_deref(),
        codex_home,
    )? {
        session.transcript_path = Some(path);
    }
    let user_marker = if session.engine == Engine::Claude {
        load_user_marker(session_id)?
    } else {
        None
    };
    let decisions = (loaders.decisions)(session_id)?;
    let queued_prompts = (loaders.queued_prompts)(session_id)?;
    let peer_out_messages = (loaders.peer_out_messages)(session_id)?;
    let proposals = (loaders.proposals)(session_id)?;
    let proposal_items = proposals
        .iter()
        .map(proposal_chat_item)
        .collect::<Result<Vec<_>>>()?;
    let todos = (loaders.todos)(session_id)?;
    let agent_files = (loaders.agent_files)(session_id)?;
    let permission_denials = (loaders.permission_denials)(session_id)?;
    let mut additional_cards = permission_denied_cards(&permission_denials);
    let parent = parent_session(&session, &mut load_session)?;
    let main_permission_pending = session
        .pending_permission_agent_ids
        .contains(MAIN_THREAD_AGENT_KEY);
    let main_permission_request_id = session
        .pending_permission_request_ids
        .get(MAIN_THREAD_AGENT_KEY)
        .cloned();
    let main_permission_data = main_permission_request_id
        .as_deref()
        .map(|request_id| (loaders.permission_request_data)(session_id, request_id))
        .transpose()?
        .flatten();
    let main_permission_is_question = main_permission_data
        .as_ref()
        .and_then(|data| data.tool_name.as_deref())
        == Some("AskUserQuestion")
        || (main_permission_data.is_none() && is_ask_user_question(&session));
    let pending_tool = if main_permission_pending && !main_permission_is_question {
        main_permission_data.as_ref()
    } else {
        None
    };
    let cache_key = MessageCacheKey::new(
        &session,
        parent.clone(),
        user_marker.clone(),
        decisions.clone(),
        queued_prompts.clone(),
        peer_out_messages.clone(),
        MessageCacheRecords {
            proposals: proposals.clone(),
            todos: todos.clone(),
            agent_files: agent_files.clone(),
            permission_denials: permission_denials.clone(),
        },
    )?;
    if let Some(mut cached) = loaders.message_cache.get(session_id, &cache_key)? {
        cached.conversation.initial_epoch =
            tail_window_limit.is_some() && session.engine == Engine::Claude;
        let elapsed = cached.cached_at.elapsed().as_secs();
        for task in &mut cached.conversation.background_tasks {
            task.elapsed_seconds = task.elapsed_seconds.saturating_add(elapsed);
        }
        cache::refresh_image_availability(&mut cached.conversation);
        if let Some(path) = session.transcript_path.as_deref() {
            subagent_reports::add_descriptions(path, &mut cached.conversation.messages);
        }
        attach_question_request_if_present(
            &mut cached.conversation.messages,
            main_permission_request_id.as_deref(),
            main_permission_data.as_ref(),
        );
        resolve_peer_session_titles(&mut cached.conversation.messages, &mut load_session)?;
        cached.conversation.update_metadata();
        cached
            .conversation
            .background_tasks
            .extend(loaders.background_tasks(session_id)?);
        if mark_resolved_queued_prompts {
            cache::mark_resolved_queued_prompts(
                loaders,
                session_id,
                &cached.resolved_queued_prompt_ids,
            );
        }
        return Ok(Some(cached.conversation));
    }

    let pending_permissions = collect_pending_permission_cards(
        session_id,
        &session,
        main_permission_is_question,
        main_permission_request_id.as_deref(),
        main_permission_data.as_ref(),
        loaders,
    )?;
    additional_cards.extend(pending_permissions.cards);

    let transcript_input = TranscriptInput {
        path: session.transcript_path.as_deref(),
        engine: session.engine,
        additions: parser::claude::ParserAdditions {
            session_id,
            parent: parent.as_ref(),
            pending_tool,
            user_marker: user_marker.as_deref(),
            decisions: &decisions,
            queued_prompts: &queued_prompts,
            peer_out_messages: &peer_out_messages,
            proposals: &proposal_items,
            todos: &todos,
            additional_cards: &additional_cards,
        },
    };
    let pending_task_ids = session
        .pending_bg_task_ids
        .union(&session.pending_agent_task_ids)
        .cloned()
        .collect::<Vec<_>>();
    let (mut conversation, resolved_queued_prompt_ids, used_tail_window) = parse_initial_or_full(
        session_id,
        transcript_input,
        tail_window_limit,
        &pending_task_ids,
        cache_key.transcript_fingerprint.clone(),
        &loaders.message_cache,
    )?;
    cache::refresh_image_availability(&mut conversation);
    resolve_peer_session_titles(&mut conversation.messages, &mut load_session)?;
    let tail_window_start = conversation.tail_window_start;
    let recent_agent_files =
        filter_after_tail_window(&agent_files, tail_window_start, |file| file.created_at);
    let storage_root = if recent_agent_files
        .iter()
        .any(|file| file.mime_type.is_none())
    {
        (loaders.agent_file_storage_root)()
    } else {
        None
    };
    merge_stored_files(
        session_id,
        &recent_agent_files,
        storage_root.as_deref(),
        &mut conversation.messages,
    );

    attach_question_request_if_present(
        &mut conversation.messages,
        main_permission_request_id.as_deref(),
        main_permission_data.as_ref(),
    );

    attach_permission_request_id(
        &mut conversation.messages,
        main_permission_request_id.as_deref(),
    );

    conversation.messages.extend(pending_permissions.questions);

    conversation.update_metadata();
    if used_tail_window {
        conversation.decisions = decisions.iter().cloned().map(ChatItem::from).collect();
        conversation.has_unanswered_proposal = proposals
            .iter()
            .any(|proposal| proposal.answer.is_none() && proposal.resolved_at.is_none());
    }
    conversation.initial_epoch = tail_window_limit.is_some() && session.engine == Engine::Claude;

    supplement_skill_suggestions(&mut conversation, &session, loaders);

    if mark_resolved_queued_prompts {
        cache::mark_resolved_queued_prompts(loaders, session_id, &resolved_queued_prompt_ids);
    }
    if !used_tail_window {
        loaders.message_cache.insert(
            session_id,
            cache_key,
            conversation.clone(),
            resolved_queued_prompt_ids,
        );
    }
    conversation
        .background_tasks
        .extend(loaders.background_tasks(session_id)?);
    Ok(Some(conversation))
}

fn supplement_skill_suggestions(
    conversation: &mut ConversationResponse,
    session: &Session,
    loaders: &RouteLoaders,
) {
    if !conversation.skills.is_empty() {
        return;
    }

    let codex_home = crate::shared::dirs::codex_home_dir();
    match skill_listing_for_session(session, codex_home.as_deref()) {
        Ok(Some(skills)) => {
            conversation.skills = skills;
            return;
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(%error, "failed to inspect current transcript skill listing"),
    }

    let mut candidates = match loaders.sessions() {
        Ok(candidates) => candidates,
        Err(error) => {
            tracing::warn!(%error, "failed to load sessions for skill suggestions");
            return;
        }
    };
    candidates.retain(|candidate| {
        candidate.session_id != session.session_id && candidate.engine == session.engine
    });
    candidates.sort_by(|left, right| {
        (left.cwd != session.cwd)
            .cmp(&(right.cwd != session.cwd))
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| right.created_at.cmp(&left.created_at))
    });

    for candidate in candidates {
        match skill_listing_for_session(&candidate, codex_home.as_deref()) {
            Ok(Some(skills)) => {
                conversation.skills = skills;
                return;
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(
                session_id = %candidate.session_id,
                %error,
                "failed to inspect candidate transcript skill listing"
            ),
        }
    }
}

fn skill_listing_for_session(
    session: &Session,
    codex_home: Option<&Path>,
) -> Result<Option<Vec<super::SkillSuggestion>>> {
    let archived_path = archived_codex_transcript_path(
        session.engine,
        session.transcript_path.as_deref(),
        codex_home,
    )?;
    let Some(path) = archived_path
        .as_deref()
        .or(session.transcript_path.as_deref())
    else {
        return Ok(None);
    };
    let Some(reader) = super::open_transcript(path, true)? else {
        return Ok(None);
    };
    Ok(super::parser::skill_suggestions::read_transcript_listing(
        reader,
        session.engine,
    )?)
}

fn attach_permission_request_id(messages: &mut [ChatItem], request_id: Option<&str>) {
    let Some(request_id) = request_id else {
        return;
    };
    for message in messages {
        if let ChatItem::Permission {
            request_id: current,
            ..
        }
        | ChatItem::PermissionWithThinking {
            request_id: current,
            ..
        } = message
            && current.is_none()
        {
            *current = Some(request_id.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::Write;

    use serde_json::json;

    use super::load_conversation_with_queued_prompts;
    use crate::commands::agent::store::PermissionRequestData;
    use crate::commands::agent::types::{Engine, MAIN_THREAD_AGENT_KEY, Session};
    use crate::commands::agent::watch_web::messages::RouteLoaders;

    #[test]
    fn conversation_load_attaches_request_id_to_grouped_thinking_permission() {
        let directory = tempfile::tempdir().unwrap();
        let transcript_path = directory.path().join("fixture-session.jsonl");
        let records = [
            json!({
                "type": "assistant",
                "thinkingDurationMs": 63,
                "message": {
                    "content": [{
                        "type": "thinking",
                        "thinking": "Review the fictional request.",
                    }],
                },
            }),
            json!({
                "type": "assistant",
                "message": {
                    "content": [{
                        "type": "tool_use",
                        "id": "fixture-pending-tool-id",
                        "name": "Bash",
                        "input": {"command": "echo awaiting approval"},
                    }],
                },
            }),
        ];
        let mut transcript_file = std::fs::File::create(&transcript_path).unwrap();
        for record in records {
            writeln!(transcript_file, "{record}").unwrap();
        }

        let mut session: Session = serde_json::from_value(json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/project",
            "status": "ended",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
        }))
        .unwrap();
        session.engine = Engine::Claude;
        session.transcript_path = Some(transcript_path);
        session.pending_permission_agent_ids = BTreeSet::from([MAIN_THREAD_AGENT_KEY.to_string()]);
        session.pending_permission_request_ids = BTreeMap::from([(
            MAIN_THREAD_AGENT_KEY.to_string(),
            "permission-request-fixture".to_string(),
        )]);

        let loaders = RouteLoaders::new(
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_| Ok(Vec::new()),
            |_, _| Ok(()),
            |_| Ok(Vec::new()),
        )
        .with_permission_request_data_loader(|_, _| {
            Ok(Some(PermissionRequestData {
                tool_name: Some("Bash".to_string()),
                tool_input: json!({"command": "echo awaiting approval"}),
                pending: true,
            }))
        });
        let conversation = load_conversation_with_queued_prompts(
            "fixture-session",
            |id| Ok((id == "fixture-session").then(|| session.clone())),
            |_| Ok(None),
            &loaders,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            serde_json::to_value(conversation.messages).unwrap(),
            json!([{
                "type": "permission",
                "requestId": "permission-request-fixture",
                "tool": "Bash",
                "description": "",
                "command": "echo awaiting approval",
                "items": [
                    {
                        "type": "thinking",
                        "text": "Review the fictional request.",
                        "thinkingDurationMs": 63,
                    },
                    {
                        "type": "tool_use",
                        "tool": "Bash",
                        "failed": false,
                    },
                ],
            }]),
        );
    }
}
