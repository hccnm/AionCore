use agent_client_protocol::schema::AvailableCommand;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Data for the `AgentStatus` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStatusEventData {
    pub backend: String,
    pub status: String,
    #[serde(default)]
    pub agent_name: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

/// Data for the `Thinking` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingEventData {
    pub content: String,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub duration: Option<u64>,
    #[serde(default)]
    pub status: Option<String>,
}

/// Data for the `Plan` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanEventData {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub entries: Vec<serde_json::Value>,
}

/// Data for the `AvailableCommands` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailableCommandsEventData {
    pub commands: Vec<AvailableCommand>,
}

/// A Claude dynamic workflow run update promoted from ACP `_meta`.
///
/// The payload intentionally keeps the raw workflow object so provider-specific
/// fields can flow through while Aion adds stable UI hints such as phases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowUpdateEventData {
    #[serde(default)]
    pub session_id: Option<String>,
    pub workflow: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_message_subtype: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowPhaseData {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Data for the `SkillSuggest` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSuggestEventData {
    #[serde(default)]
    pub cron_job_id: Option<String>,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub skill_content: Option<String>,
}

/// Data for the `CronTrigger` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronTriggerEventData {
    pub cron_job_id: String,
    pub cron_job_name: String,
    pub triggered_at: i64,
}
