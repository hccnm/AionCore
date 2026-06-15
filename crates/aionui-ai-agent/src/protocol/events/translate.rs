use agent_client_protocol::schema::{
    ContentBlock, ExtNotification, Meta as SdkMeta, PermissionOption, PermissionOptionKind as SdkPermissionOptionKind,
    RequestPermissionRequest, SessionNotification, SessionUpdate, ToolCallContent as SdkToolCallContent,
    ToolCallLocation as SdkToolCallLocation, ToolCallStatus as SdkToolCallStatus, ToolCallUpdate as SdkToolCallUpdate,
    ToolKind as SdkToolKind,
};
use regex::Regex;
use serde_json::{Map, Value, json};
use tracing::debug;

use super::permission::{
    AcpPermissionEventData, AcpPermissionOptionData, AcpPermissionOptionKind, AcpPermissionRequestData,
    AcpPermissionToolCall,
};
use super::session_updates::{
    AvailableCommandsEventData, PlanEventData, ThinkingEventData, WorkflowPhaseData, WorkflowUpdateEventData,
};
use super::tool_call::{
    AcpToolCallContentItem, AcpToolCallEventData, AcpToolCallKind, AcpToolCallLocationItem,
    AcpToolCallSessionUpdateKind, AcpToolCallStatus, AcpToolCallTextBlock, AcpToolCallTextBlockType,
    AcpToolCallUpdateData,
};
use super::{AgentStreamEvent, TextEventData};

/// Convert an SDK [`SessionNotification`] into zero or more [`AgentStreamEvent`]s.
pub(crate) fn session_notification_to_events(notif: &SessionNotification) -> Vec<AgentStreamEvent> {
    let session_id = notif.session_id.to_string();
    let mut events = Vec::new();

    match &notif.update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            if let ContentBlock::Text(text) = &chunk.content {
                events.push(AgentStreamEvent::Text(TextEventData {
                    content: text.text.clone(),
                }));
            }
        }

        SessionUpdate::AgentThoughtChunk(chunk) => {
            if let ContentBlock::Text(text) = &chunk.content {
                events.push(AgentStreamEvent::Thinking(ThinkingEventData {
                    content: text.text.clone(),
                    subject: None,
                    duration: None,
                    status: Some("in_progress".into()),
                }));
            }
        }

        SessionUpdate::UserMessageChunk(_chunk) => {}

        SessionUpdate::ToolCall(tc) => {
            let workflow_event = workflow_update_from_meta(&session_id, tc.meta.as_ref(), "tool_call");
            events.push(AgentStreamEvent::AcpToolCall(AcpToolCallEventData {
                session_id,
                update: AcpToolCallUpdateData {
                    session_update: AcpToolCallSessionUpdateKind::ToolCall,
                    tool_call_id: tc.tool_call_id.to_string(),
                    status: Some(map_sdk_tool_status(&tc.status)),
                    title: Some(tc.title.clone()),
                    kind: Some(map_sdk_tool_kind(&tc.kind)),
                    raw_input: tc.raw_input.clone(),
                    raw_output: None,
                    content: map_tool_call_content(&tc.content),
                    locations: map_tool_call_locations(&tc.locations),
                },
                meta: tc.meta.clone(),
            }));
            if let Some(event) = workflow_event {
                events.push(AgentStreamEvent::WorkflowUpdate(event));
            }
        }

        SessionUpdate::ToolCallUpdate(tcu) => {
            let workflow_event = workflow_update_from_meta(&session_id, tcu.meta.as_ref(), "tool_call_update");
            events.push(AgentStreamEvent::AcpToolCall(AcpToolCallEventData {
                session_id,
                update: AcpToolCallUpdateData {
                    session_update: AcpToolCallSessionUpdateKind::ToolCallUpdate,
                    tool_call_id: tcu.tool_call_id.to_string(),
                    status: tcu.fields.status.as_ref().map(map_sdk_tool_status),
                    title: tcu.fields.title.clone(),
                    kind: tcu.fields.kind.as_ref().map(map_sdk_tool_kind),
                    raw_input: tcu.fields.raw_input.clone(),
                    raw_output: tcu.fields.raw_output.clone(),
                    content: tcu
                        .fields
                        .content
                        .as_ref()
                        .and_then(|content| map_tool_call_content(content)),
                    locations: tcu
                        .fields
                        .locations
                        .as_ref()
                        .and_then(|locations| map_tool_call_locations(locations)),
                },
                meta: tcu.meta.clone(),
            }));
            if let Some(event) = workflow_event {
                events.push(AgentStreamEvent::WorkflowUpdate(event));
            }
        }

        SessionUpdate::Plan(plan) => {
            let entries: Vec<serde_json::Value> = plan
                .entries
                .iter()
                .map(|e| serde_json::to_value(e).unwrap_or_default())
                .collect();

            events.push(AgentStreamEvent::Plan(PlanEventData {
                session_id: Some(session_id),
                entries,
            }));
        }

        SessionUpdate::AvailableCommandsUpdate(update) => {
            events.push(AgentStreamEvent::AvailableCommands(AvailableCommandsEventData {
                commands: update.available_commands.clone(),
            }));
        }

        SessionUpdate::CurrentModeUpdate(update) => {
            events.push(AgentStreamEvent::AcpModeInfo(
                serde_json::to_value(update).unwrap_or_default(),
            ));
        }

        SessionUpdate::ConfigOptionUpdate(update) => {
            events.push(AgentStreamEvent::AcpConfigOption(
                serde_json::to_value(update).unwrap_or_default(),
            ));
        }

        SessionUpdate::SessionInfoUpdate(update) => {
            events.push(AgentStreamEvent::AcpSessionInfo(
                serde_json::to_value(update).unwrap_or_default(),
            ));
        }

        SessionUpdate::UsageUpdate(update) => {
            events.push(AgentStreamEvent::AcpContextUsage(
                serde_json::to_value(update).unwrap_or_default(),
            ));
        }
        _ => {
            debug!("Unknown SessionUpdate variant received, skipping");
        }
    }

    events
}

/// Convert an ACP extension notification into stream events.
pub(crate) fn ext_notification_to_events(notification: &ExtNotification) -> Vec<AgentStreamEvent> {
    match notification.method.as_ref() {
        "claude/workflowUpdate" | "_claude/workflowUpdate" => workflow_update_from_ext_notification(notification)
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn permission_request_to_event_data(request: &RequestPermissionRequest) -> AcpPermissionEventData {
    AcpPermissionEventData::Request(AcpPermissionRequestData {
        session_id: request.session_id.to_string(),
        tool_call: map_permission_tool_call(&request.tool_call),
        options: request.options.iter().map(map_permission_option).collect(),
        meta: request.meta.clone(),
    })
}

fn map_sdk_tool_status(sdk: &SdkToolCallStatus) -> AcpToolCallStatus {
    match sdk {
        SdkToolCallStatus::Pending => AcpToolCallStatus::Pending,
        SdkToolCallStatus::InProgress => AcpToolCallStatus::InProgress,
        SdkToolCallStatus::Completed => AcpToolCallStatus::Completed,
        SdkToolCallStatus::Failed => AcpToolCallStatus::Failed,
        _ => AcpToolCallStatus::Pending,
    }
}

fn map_sdk_tool_kind(kind: &SdkToolKind) -> AcpToolCallKind {
    match kind {
        SdkToolKind::Read | SdkToolKind::Search => AcpToolCallKind::Read,
        SdkToolKind::Edit | SdkToolKind::Delete | SdkToolKind::Move => AcpToolCallKind::Edit,
        SdkToolKind::Execute
        | SdkToolKind::Think
        | SdkToolKind::Fetch
        | SdkToolKind::SwitchMode
        | SdkToolKind::Other
        | _ => AcpToolCallKind::Execute,
    }
}

fn map_sdk_permission_option_kind(kind: SdkPermissionOptionKind) -> AcpPermissionOptionKind {
    match kind {
        SdkPermissionOptionKind::AllowOnce => AcpPermissionOptionKind::AllowOnce,
        SdkPermissionOptionKind::AllowAlways => AcpPermissionOptionKind::AllowAlways,
        SdkPermissionOptionKind::RejectOnce => AcpPermissionOptionKind::RejectOnce,
        SdkPermissionOptionKind::RejectAlways => AcpPermissionOptionKind::RejectAlways,
        _ => AcpPermissionOptionKind::RejectOnce,
    }
}

fn map_permission_tool_call(tool_call: &SdkToolCallUpdate) -> AcpPermissionToolCall {
    AcpPermissionToolCall {
        tool_call_id: tool_call.tool_call_id.to_string(),
        status: tool_call.fields.status.as_ref().map(map_sdk_tool_status),
        title: tool_call.fields.title.clone(),
        kind: tool_call.fields.kind.as_ref().map(map_sdk_tool_kind),
        raw_input: tool_call.fields.raw_input.clone(),
        raw_output: tool_call.fields.raw_output.clone(),
        content: tool_call
            .fields
            .content
            .as_ref()
            .and_then(|content| map_tool_call_content(content)),
        locations: tool_call
            .fields
            .locations
            .as_ref()
            .and_then(|locations| map_tool_call_locations(locations)),
        meta: tool_call.meta.clone(),
    }
}

fn map_permission_option(option: &PermissionOption) -> AcpPermissionOptionData {
    AcpPermissionOptionData {
        option_id: option.option_id.to_string(),
        name: option.name.clone(),
        kind: map_sdk_permission_option_kind(option.kind),
        meta: option.meta.clone(),
    }
}

fn map_tool_call_content(content: &[SdkToolCallContent]) -> Option<Vec<AcpToolCallContentItem>> {
    let items: Vec<AcpToolCallContentItem> = content
        .iter()
        .filter_map(|item| match item {
            SdkToolCallContent::Content(content) => match &content.content {
                ContentBlock::Text(text) => Some(AcpToolCallContentItem::Content {
                    content: AcpToolCallTextBlock {
                        block_type: AcpToolCallTextBlockType::Text,
                        text: text.text.clone(),
                    },
                }),
                _ => None,
            },
            SdkToolCallContent::Diff(diff) => Some(AcpToolCallContentItem::Diff {
                path: diff.path.to_string_lossy().into_owned(),
                old_text: diff.old_text.clone(),
                new_text: diff.new_text.clone(),
            }),
            SdkToolCallContent::Terminal(_) => None,
            _ => None,
        })
        .collect();

    if items.is_empty() { None } else { Some(items) }
}

fn map_tool_call_locations(locations: &[SdkToolCallLocation]) -> Option<Vec<AcpToolCallLocationItem>> {
    (!locations.is_empty()).then(|| {
        locations
            .iter()
            .map(|loc| AcpToolCallLocationItem {
                path: loc.path.to_string_lossy().into_owned(),
            })
            .collect()
    })
}

fn workflow_update_from_meta(
    session_id: &str,
    meta: Option<&SdkMeta>,
    source_message_subtype: &str,
) -> Option<WorkflowUpdateEventData> {
    let meta = meta?;
    let claude_meta = meta.get("claudeCode").or_else(|| meta.get("claude_code"))?;
    let workflow = claude_meta.get("workflow")?;
    let mut workflow = workflow.clone();
    enrich_workflow_phases(&mut workflow);
    debug_workflow_update(&workflow, source_message_subtype);

    Some(WorkflowUpdateEventData {
        session_id: Some(session_id.to_owned()),
        workflow,
        runs: claude_meta.get("runs").cloned(),
        source_message_subtype: Some(source_message_subtype.to_owned()),
    })
}

fn workflow_update_from_ext_notification(notification: &ExtNotification) -> Option<AgentStreamEvent> {
    let value = serde_json::from_str::<Value>(notification.params.get()).ok()?;
    let workflow = value.get("workflow")?.clone();
    let mut workflow = workflow;
    enrich_workflow_phases(&mut workflow);
    debug_workflow_update(&workflow, "ext_notification");

    Some(AgentStreamEvent::WorkflowUpdate(WorkflowUpdateEventData {
        session_id: string_from_value(&value, &["sessionId", "session_id"]),
        workflow,
        runs: value.get("runs").cloned(),
        source_message_subtype: string_from_value(&value, &["sourceMessageSubtype", "source_message_subtype"])
            .or_else(|| Some("ext_notification".to_owned())),
    }))
}

fn string_from_value(value: &Value, keys: &[&str]) -> Option<String> {
    let obj = value.as_object()?;
    string_from_object(obj, keys)
}

fn enrich_workflow_phases(workflow: &mut Value) {
    let Some(obj) = workflow.as_object_mut() else {
        return;
    };
    if obj.get("phases").is_some() {
        return;
    }
    let script = if let Some(inline_script) = string_from_nested_object(
        obj,
        &["rawInput", "raw_input"],
        &["script", "workflowScript", "workflow_script"],
    ) {
        inline_script
    } else {
        let Some(script_path) = string_from_object(obj, &["scriptPath", "script_path"])
            .or_else(|| string_from_nested_object(obj, &["rawInput", "raw_input"], &["scriptPath", "script_path"]))
        else {
            return;
        };
        let Ok(script) = std::fs::read_to_string(script_path) else {
            return;
        };
        script
    };
    let phases = parse_workflow_phases(&script);
    if phases.is_empty() {
        return;
    }
    let phase_values = phases
        .into_iter()
        .map(|phase| serde_json::to_value(phase).unwrap_or_else(|_| json!({})))
        .collect();
    obj.insert("phases".to_owned(), Value::Array(phase_values));
}

fn debug_workflow_update(workflow: &Value, source: &str) {
    let Some(obj) = workflow.as_object() else {
        return;
    };
    let first_agent = obj
        .get("workflowAgents")
        .or_else(|| obj.get("workflow_agents"))
        .and_then(|value| value.as_array())
        .and_then(|agents| agents.first())
        .and_then(Value::as_object);
    debug!(
        source,
        workflow_name = ?string_from_object(obj, &["workflowName", "workflow_name", "name"]),
        task_id = ?string_from_object(obj, &["taskId", "task_id"]),
        run_id = ?string_from_object(obj, &["runId", "run_id"]),
        current_phase = ?string_from_object(obj, &["currentPhase", "current_phase"]),
        last_tool = ?string_from_object(obj, &["lastToolName", "last_tool_name"]),
        phase_count = obj.get("phases").and_then(|value| value.as_array()).map_or(0, Vec::len),
        agent_count = obj
            .get("workflowAgents")
            .or_else(|| obj.get("workflow_agents"))
            .and_then(|value| value.as_array())
            .map_or(0, Vec::len),
        first_agent_label = ?first_agent.and_then(|agent| string_from_object(agent, &["label", "name", "lastToolName", "last_tool_name"])),
        first_agent_action = ?first_agent.and_then(|agent| string_from_object(agent, &["currentAction", "current_action", "description", "summary"])),
        first_agent_phase = ?first_agent.and_then(|agent| string_from_object(agent, &["phase", "phaseTitle", "phase_title", "currentPhase", "current_phase"])),
        "workflow update translated"
    );
}

fn string_from_object(obj: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| obj.get(*key).and_then(Value::as_str))
        .map(str::to_owned)
}

fn string_from_nested_object(obj: &Map<String, Value>, parents: &[&str], keys: &[&str]) -> Option<String> {
    parents
        .iter()
        .find_map(|parent| obj.get(*parent).and_then(Value::as_object))
        .and_then(|nested| string_from_object(nested, keys))
}

fn parse_workflow_phases(script: &str) -> Vec<WorkflowPhaseData> {
    let phases_re = Regex::new(r#"(?s)phases\s*:\s*\[(?P<body>.*?)\]\s*[,}]"#).expect("valid phases regex");
    let Some(body) = phases_re
        .captures(script)
        .and_then(|captures| captures.name("body"))
        .map(|m| m.as_str())
    else {
        return Vec::new();
    };

    let object_re = Regex::new(r#"(?s)\{(?P<body>.*?)\}"#).expect("valid phase object regex");
    object_re
        .captures_iter(body)
        .filter_map(|captures| {
            let body = captures.name("body")?.as_str();
            let title = capture_js_string_property(body, "title")?;
            let detail = capture_js_string_property(body, "detail");
            Some(WorkflowPhaseData { title, detail })
        })
        .collect()
}

fn capture_js_string_property(body: &str, property: &str) -> Option<String> {
    let re = Regex::new(&format!(
        r#"{}\s*:\s*['"`](?P<value>[^'"`]+)['"`]"#,
        regex::escape(property)
    ))
    .ok()?;
    re.captures(body)
        .and_then(|captures| captures.name("value"))
        .map(|m| m.as_str().to_owned())
}
