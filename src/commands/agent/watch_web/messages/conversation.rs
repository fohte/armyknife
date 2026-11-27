use std::{collections::HashSet, path::Path};

use anyhow::Result;

use crate::commands::agent::store::WatchWebQueuedPrompt;
use crate::commands::agent::types::{Engine, MAIN_THREAD_AGENT_KEY, Session};
use crate::commands::agent::user_turn::is_ask_user_question;

use super::cache::{MessageCacheKey, MessageCacheRecords};
use super::file_attachments::merge_stored_files;
use super::incremental::{
    TranscriptRangeContext, filter_after_tail_window, parse_initial_or_full,
    parse_initial_or_full_for_range,
};
use super::pending_permissions::{collect_pending_permission_cards, permission_denied_cards};
use super::proposals::proposal_chat_item;
use super::questions::attach_question_request_if_present;
use super::routes::RouteLoaders;
use super::session_titles::{parent_session, resolve_peer_session_titles};
use super::subagent_reports;
use super::transcript_path::archived_codex_transcript_path;
use super::{ChatItem, ConversationResponse, TranscriptInput, cache, command_source, parser};

struct ConversationLoadOptions<'a> {
    tail_window_limit: Option<usize>,
    mark_resolved_queued_prompts: bool,
    range: TranscriptRangeContext<'a>,
}

pub(super) fn load_conversation_with_queued_prompts(
    session_id: &str,
    load_session: impl FnMut(&str) -> Result<Option<Session>>,
    load_user_marker: impl FnOnce(&str) -> Result<Option<String>>,
    loaders: &RouteLoaders,
) -> Result<Option<ConversationResponse>> {
    load_conversation_with_queued_prompts_for_range(
        session_id,
        load_session,
        load_user_marker,
        loaders,
        None,
        None,
        false,
    )
}

pub(super) fn load_conversation_with_queued_prompts_for_range(
    session_id: &str,
    load_session: impl FnMut(&str) -> Result<Option<Session>>,
    load_user_marker: impl FnOnce(&str) -> Result<Option<String>>,
    loaders: &RouteLoaders,
    requested_epoch: Option<&str>,
    older_page_limit: Option<usize>,
    require_full_history: bool,
) -> Result<Option<ConversationResponse>> {
    load_conversation_with_tail_window_and_marker_updates(
        session_id,
        load_session,
        load_user_marker,
        loaders,
        ConversationLoadOptions {
            tail_window_limit: None,
            mark_resolved_queued_prompts: true,
            range: TranscriptRangeContext {
                requested_epoch,
                older_page_limit,
                require_full_history,
            },
        },
    )
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
        ConversationLoadOptions {
            tail_window_limit: None,
            mark_resolved_queued_prompts: false,
            range: TranscriptRangeContext::default(),
        },
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
    load_conversation_with_tail_window_and_marker_updates(
        session_id,
        load_session,
        load_user_marker,
        loaders,
        ConversationLoadOptions {
            tail_window_limit: Some(message_limit),
            mark_resolved_queued_prompts: true,
            range: TranscriptRangeContext::default(),
        },
    )
}

fn load_conversation_with_tail_window_and_marker_updates(
    session_id: &str,
    load_session: impl FnMut(&str) -> Result<Option<Session>>,
    load_user_marker: impl FnOnce(&str) -> Result<Option<String>>,
    loaders: &RouteLoaders,
    options: ConversationLoadOptions<'_>,
) -> Result<Option<ConversationResponse>> {
    let codex_home = crate::shared::dirs::codex_home_dir();
    load_conversation_with_tail_window_from(
        session_id,
        load_session,
        load_user_marker,
        loaders,
        codex_home.as_deref(),
        options,
    )
}

fn load_conversation_with_tail_window_from(
    session_id: &str,
    mut load_session: impl FnMut(&str) -> Result<Option<Session>>,
    load_user_marker: impl FnOnce(&str) -> Result<Option<String>>,
    loaders: &RouteLoaders,
    codex_home: Option<&Path>,
    options: ConversationLoadOptions<'_>,
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
            options.tail_window_limit.is_some() && session.engine == Engine::Claude;
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
        supplement_skill_suggestions(&mut cached.conversation, &session, loaders);
        resolve_peer_session_titles(&mut cached.conversation.messages, &mut load_session)?;
        cached.conversation.update_metadata();
        cached.conversation.confirmed_queued_prompt_ids = confirmed_queued_prompt_message_ids(
            &queued_prompts,
            &cached.resolved_queued_prompt_ids,
        );
        cached
            .conversation
            .background_tasks
            .extend(loaders.background_tasks(session_id)?);
        if options.mark_resolved_queued_prompts {
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
    let parsed = if options.range.requested_epoch.is_none() && !options.range.require_full_history {
        parse_initial_or_full(
            session_id,
            transcript_input,
            options.tail_window_limit,
            &pending_task_ids,
            cache_key.transcript_fingerprint.clone(),
            &loaders.message_cache,
        )
    } else {
        parse_initial_or_full_for_range(
            session_id,
            transcript_input,
            options.tail_window_limit,
            &pending_task_ids,
            cache_key.transcript_fingerprint.clone(),
            &loaders.message_cache,
            options.range,
        )
    }?;
    let (mut conversation, resolved_queued_prompt_ids, used_tail_window) = parsed;
    conversation.confirmed_queued_prompt_ids =
        confirmed_queued_prompt_message_ids(&queued_prompts, &resolved_queued_prompt_ids);
    let used_tail_window = used_tail_window || conversation.tail_window_start.is_some();
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
    super::answer_images::attach(session_id, &mut conversation);

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
        conversation.has_unanswered_proposal = proposals
            .iter()
            .any(|proposal| proposal.answer.is_none() && proposal.resolved_at.is_none());
    }
    conversation.initial_epoch =
        options.tail_window_limit.is_some() && session.engine == Engine::Claude;

    supplement_skill_suggestions(&mut conversation, &session, loaders);

    if options.mark_resolved_queued_prompts {
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

fn confirmed_queued_prompt_message_ids(
    queued_prompts: &[WatchWebQueuedPrompt],
    newly_resolved_ids: &[i64],
) -> Vec<String> {
    let newly_resolved_ids = newly_resolved_ids.iter().copied().collect::<HashSet<_>>();
    queued_prompts
        .iter()
        .filter(|prompt| newly_resolved_ids.contains(&prompt.id))
        .filter_map(|prompt| prompt.message_id.clone())
        .collect()
}

fn supplement_skill_suggestions(
    conversation: &mut ConversationResponse,
    session: &Session,
    loaders: &RouteLoaders,
) {
    if conversation.skill_suggestions_from_transcript {
        return;
    }
    if !conversation.skill_suggestions_are_fallback && !conversation.skills.is_empty() {
        conversation.skill_suggestions_from_transcript = true;
        return;
    }
    if conversation.skill_suggestions_are_fallback {
        conversation.skills.clear();
        conversation.skill_suggestions_are_fallback = false;
    }

    let codex_home = crate::shared::dirs::codex_home_dir();
    match super::skill_suggestions::cached_skill_listing_for_session(
        session,
        loaders,
        codex_home.as_deref(),
    ) {
        Ok(Some(skills)) => {
            conversation.skills = skills;
            conversation.skill_suggestions_from_transcript = true;
            return;
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(%error, "failed to inspect current transcript skill listing"),
    }

    match super::skill_suggestions::find_from_recent_sessions(
        session.engine,
        Some(&session.cwd),
        Some(&session.session_id),
        loaders,
    ) {
        Ok(skills) if !skills.is_empty() => {
            conversation.skills = skills;
            conversation.skill_suggestions_are_fallback = true;
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(%error, "failed to load sessions for skill suggestions");
        }
    }
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
