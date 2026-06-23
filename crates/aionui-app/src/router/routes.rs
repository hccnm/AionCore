#![allow(clippy::disallowed_types)]

//! Top-level router assembly: middleware stack + module route merges.

use std::sync::Arc;
use std::time::Instant;

use axum::Json;
use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, Extension, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Router, middleware};
use tower_http::cors::{Any, CorsLayer};

use aionui_ai_agent::{agent_routes, remote_agent_routes};
use aionui_api_types::{
    ApiResponse, CloneConversationRequest, ConversationResponse, CreateConversationRequest, ErrorResponse,
};
use aionui_assets::{AssetRouterState, asset_routes};
use aionui_assistant::assistant_routes;
use aionui_auth::{AuthRouterState, AuthState, CurrentUser, auth_middleware, auth_routes, security_headers_middleware};
use aionui_channel::channel_routes;
#[cfg(feature = "weixin")]
use aionui_channel::weixin_login_route;
use aionui_conversation::{
    ConversationRouterState, conversation_ops_routes, conversation_routes, conversation_routes_without_create_or_clone,
};
use aionui_cron::cron_routes;
use aionui_db::IUserRepository;
use aionui_extension::{extension_routes, hub_routes, skill_routes};
use aionui_file::file_routes;
use aionui_mcp::mcp_routes;
use aionui_office::{office_proxy_routes, office_routes};
use aionui_realtime::{WsHandlerState, ws_upgrade_handler};
use aionui_shell::shell_routes;
use aionui_system::{connection_test_routes, system_routes};
use aionui_team::team_routes;
use serde_json::{Map, Value, json};

use crate::services::AppServices;
use crate::workbench_routes::{WorkbenchRouterState, workbench_public_routes, workbench_routes};
use crate::workspace_resolver::{WorkspaceResolveError, WorkspaceResolveMode, WorkspaceResolver};

use super::health::{guide_mcp_status, health_check};
use super::state::{ModuleStates, RouterBuildError, build_module_states, build_ws_state};
use super::trace::with_access_log;

#[derive(Clone)]
struct WorkbenchConversationCreateState {
    workbench: WorkbenchRouterState,
    conversation: ConversationRouterState,
    user_repo: Arc<dyn IUserRepository>,
}

const JSON_API_NORMALIZATION_MAX_BYTES: usize = aionui_common::constants::BODY_LIMIT;

async fn create_workbench_conversation(
    State(state): State<WorkbenchConversationCreateState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<CreateConversationRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ApiResponse<ConversationResponse>>), aionui_common::ApiError> {
    let Json(req) = body.map_err(aionui_common::ApiError::from)?;
    ensure_legacy_conversation_user(&state.user_repo, &user).await?;
    let req = prepare_workbench_conversation_request(&state.workbench, &user.id, req).await?;
    let conversation = state
        .conversation
        .service
        .create(&user.id, req)
        .await
        .map_err(aionui_common::ApiError::from)?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(conversation))))
}

async fn clone_workbench_conversation(
    State(state): State<WorkbenchConversationCreateState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<CloneConversationRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ApiResponse<ConversationResponse>>), aionui_common::ApiError> {
    let Json(req) = body.map_err(aionui_common::ApiError::from)?;
    ensure_legacy_conversation_user(&state.user_repo, &user).await?;
    let req = prepare_workbench_conversation_request(&state.workbench, &user.id, req.conversation).await?;
    let conversation = state
        .conversation
        .service
        .create(&user.id, req)
        .await
        .map_err(aionui_common::ApiError::from)?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(conversation))))
}

async fn ensure_legacy_conversation_user(
    user_repo: &Arc<dyn IUserRepository>,
    user: &CurrentUser,
) -> Result<(), aionui_common::ApiError> {
    user_repo
        .ensure_user_with_id(&user.id, &user.username)
        .await
        .map(|_| ())
        .map_err(|error| {
            tracing::error!(user_id = %user.id, error = %error, "failed to ensure legacy conversation user");
            aionui_common::ApiError::Internal("Failed to prepare conversation user".into())
        })
}

async fn prepare_workbench_conversation_request(
    workbench: &WorkbenchRouterState,
    user_id: &str,
    mut req: CreateConversationRequest,
) -> Result<CreateConversationRequest, aionui_common::ApiError> {
    let workspace_id = req
        .workspace_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| aionui_common::ApiError::BadRequest("workspace_id is required".into()))?
        .to_owned();

    if req.extra.get("workspace").is_some() {
        return Err(aionui_common::ApiError::BadRequest(
            "extra.workspace is not accepted in SaaS mode; use workspace_id".into(),
        ));
    }

    let resolver = WorkspaceResolver::new(workbench.senmo_root.clone(), workbench.workspaces.clone());
    let resolved = resolver
        .resolve_for_user(user_id, &workspace_id, "", WorkspaceResolveMode::Existing)
        .await
        .map_err(workspace_resolve_api_error)?;

    let extra = req
        .extra
        .as_object_mut()
        .ok_or_else(|| aionui_common::ApiError::BadRequest("extra must be a JSON object".into()))?;
    extra.insert("workspace_id".to_owned(), serde_json::Value::String(workspace_id));
    extra.insert(
        "workspace_relative_path".to_owned(),
        serde_json::Value::String(resolved.relative_path.clone()),
    );
    // Legacy conversation/agent code still consumes extra.workspace as its cwd.
    // The value is derived server-side from workspace_id and is not accepted
    // from clients in SaaS mode.
    extra.insert(
        "workspace".to_owned(),
        serde_json::Value::String(resolved.workspace_root.to_string_lossy().into_owned()),
    );

    Ok(req)
}

fn workspace_resolve_api_error(error: WorkspaceResolveError) -> aionui_common::ApiError {
    match error {
        WorkspaceResolveError::NotFound(message) => aionui_common::ApiError::NotFound(message),
        WorkspaceResolveError::Forbidden(message) => aionui_common::ApiError::Forbidden(message),
        WorkspaceResolveError::BadPath(message) => aionui_common::ApiError::BadRequest(message),
        WorkspaceResolveError::Lookup(message) | WorkspaceResolveError::Internal(message) => {
            tracing::error!(error = %message, "workspace path resolution failed");
            aionui_common::ApiError::Internal("Workspace path resolution failed".into())
        }
    }
}

/// Create the application router with all routes and global middleware.
///
/// Middleware stack (outermost → innermost):
/// 1. Security response headers (X-Frame-Options, etc.)
/// 2. Route handlers (auth routes + system routes + conversation routes + file routes + health check)
///
/// SaaS is the only supported remote mode. Authentication is Bearer/JWT based
/// and state-changing requests are not protected by cookie CSRF middleware.
pub async fn create_router(services: &AppServices) -> Result<Router, RouterBuildError> {
    let boot = Instant::now();
    tracing::info!("startup: router assembly started");

    // Bridge event bus → WebSocket manager: forward all broadcast events
    // to connected WebSocket clients.
    let mut event_rx = services.event_bus.subscribe();
    let ws_manager = services.ws_manager.clone();
    tokio::spawn(async move {
        while let Ok(event) = event_rx.recv().await {
            ws_manager.broadcast_all(event);
        }
    });

    let (states, channel_components) = build_module_states(services).await?;
    tracing::info!(elapsed_ms = boot.elapsed().as_millis(), "startup: module states built");

    // Wire TeamSessionService into Guide MCP server now that both are available.
    services
        .inject_guide_service(Arc::downgrade(&states.team.service))
        .await;
    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: guide MCP service injected"
    );

    // Start channel orchestrator (message loop)
    tokio::spawn(
        channel_components
            .orchestrator
            .run(channel_components.message_rx, channel_components.confirm_rx),
    );
    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: channel orchestrator spawned"
    );

    // Restore enabled channel plugins (starts receiving IM messages)
    let chan_mgr = channel_components.manager;
    let chan_factory = channel_components.plugin_factory;
    tokio::spawn(async move {
        if let Err(e) = chan_mgr.restore_plugins(&chan_factory).await {
            tracing::warn!(
                code = "BOOTSTRAP_DEGRADED_CHANNEL_RESTORE",
                stage = "channel.restore",
                error = %e,
                "failed to restore channel plugins"
            );
        }
    });
    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: channel plugin restore scheduled"
    );

    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: route tree build started"
    );
    let router = create_router_with_states(services, states);
    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: router assembly completed"
    );
    Ok(router)
}

/// Create the application router with custom module states.
///
/// Used for testing when specific service overrides are needed
/// (e.g. injecting a mock HTTP server URL for version check).
pub fn create_router_with_states(services: &AppServices, states: ModuleStates) -> Router {
    let ws_state = build_ws_state(services);
    create_router_with_all_state(services, states, ws_state)
}

/// Create the application router with custom module states and WebSocket state.
///
/// Full-control variant used by tests that need to override
/// module services and WebSocket behaviour.
pub fn create_router_with_all_state(services: &AppServices, states: ModuleStates, ws_state: WsHandlerState) -> Router {
    let boot = Instant::now();
    tracing::info!("startup: route tree build with states started");

    let auth_state = AuthRouterState {
        jwt_service: services.jwt_service.clone(),
        user_repo: services.user_repo.clone(),
        platform_user_repo: services.workbench.as_ref().map(|repos| repos.users.clone()),
        external_identity_repo: services
            .workbench
            .as_ref()
            .map(|repos| repos.external_identities.clone()),
        role_repo: services.workbench.as_ref().map(|repos| repos.roles.clone()),
        gateway_auth: services.gateway_auth.clone(),
        cookie_config: services.cookie_config.clone(),
        qr_token_store: services.qr_token_store.clone(),
        local: services.local,
    };

    let auth_mw_state = AuthState {
        jwt_service: services.jwt_service.clone(),
        user_repo: services.user_repo.clone(),
        platform_user_repo: services.workbench.as_ref().map(|repos| repos.users.clone()),
        external_identity_repo: services
            .workbench
            .as_ref()
            .map(|repos| repos.external_identities.clone()),
        gateway_auth: services.gateway_auth.clone(),
        local: services.local,
    };

    // System routes protected by auth middleware
    let system_authenticated =
        system_routes(states.system).route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    let workbench_state_raw = match (&services.workbench, &services.senmo_workspace_root) {
        (Some(repos), Some(senmo_root)) => Some(WorkbenchRouterState {
            users: repos.users.clone(),
            external_identities: repos.external_identities.clone(),
            roles: repos.roles.clone(),
            git_ssh_credentials: repos.git_ssh_credentials.clone(),
            git_projects: repos.git_projects.clone(),
            workspaces: repos.workspaces.clone(),
            snapshots: repos.snapshots.clone(),
            execution_runs: repos.execution_runs.clone(),
            execution_artifacts: repos.execution_artifacts.clone(),
            audit_logs: repos.audit_logs.clone(),
            senmo_root: senmo_root.clone(),
            gateway_auth: services.gateway_auth.clone(),
            encryption_key: crate::config::derive_encryption_key(&services.jwt_secret_raw),
            git_ops: std::sync::Arc::new(crate::workbench_routes::DefaultGitProjectOps),
        }),
        _ => None,
    };

    // Conversation routes protected by auth middleware. In SaaS mode, creation
    // is wrapped here so the request must carry workspace_id and never treats
    // client-supplied extra.workspace as the source of truth.
    let conversation_authenticated = match workbench_state_raw.clone() {
        Some(workbench) => {
            let create_state = WorkbenchConversationCreateState {
                workbench,
                conversation: states.conversation.clone(),
                user_repo: services.user_repo.clone(),
            };
            Router::new()
                .route("/api/conversations", post(create_workbench_conversation))
                .route("/api/conversations/clone", post(clone_workbench_conversation))
                .with_state(create_state)
                .merge(conversation_routes_without_create_or_clone(states.conversation.clone()))
        }
        None => conversation_routes(states.conversation.clone()),
    }
    .route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    let conversation_ops_authenticated = conversation_ops_routes(states.conversation)
        .route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    // Remote agent routes protected by auth middleware
    let remote_agent_authenticated = remote_agent_routes(states.remote_agent)
        .route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    // Unified agent listing/refresh/test routes protected by auth middleware
    let agent_authenticated =
        agent_routes(states.agent).route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    // Connection test routes (Bedrock, Gemini) protected by auth middleware
    let connection_test_authenticated = connection_test_routes(states.connection_test)
        .route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    // File routes protected by auth middleware. SaaS mode exposes the
    // workspace-scoped file API from workbench_routes instead of legacy arbitrary
    // path endpoints.
    let file_authenticated = if workbench_state_raw.is_some() {
        Router::new()
    } else {
        file_routes(states.file).route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware))
    };

    // MCP routes protected by auth middleware
    let mcp_authenticated =
        mcp_routes(states.mcp).route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    // Extension routes protected by auth middleware
    let extension_authenticated =
        extension_routes(states.extension).route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    // Hub routes protected by auth middleware
    let hub_authenticated =
        hub_routes(states.hub).route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    // Skill routes protected by auth middleware
    let skill_authenticated =
        skill_routes(states.skill).route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    // Channel routes protected by auth middleware
    #[cfg(feature = "weixin")]
    let weixin_login_authenticated = weixin_login_route(states.channel.clone())
        .route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));
    let channel_authenticated =
        channel_routes(states.channel).route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    // Team routes protected by auth middleware
    let team_authenticated =
        team_routes(states.team).route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    // Cron routes protected by auth middleware
    let cron_authenticated =
        cron_routes(states.cron).route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    // Office routes protected by auth middleware
    let office_authenticated =
        office_routes(states.office.clone()).route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    // Shell + STT routes protected by auth middleware
    let shell_authenticated =
        shell_routes(states.shell).route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    // Assistant routes protected by auth middleware (T1a skeleton: all
    // handlers return 500 "not implemented"; T1b wires real service)
    let assistant_authenticated =
        assistant_routes(states.assistant).route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware));

    let workbench_state = workbench_state_raw
        .clone()
        .map(|state| workbench_routes(state).route_layer(from_fn_with_state(auth_mw_state.clone(), auth_middleware)))
        .unwrap_or_else(Router::new);
    let workbench_public = workbench_state_raw
        .clone()
        .map(workbench_public_routes)
        .unwrap_or_else(Router::new);

    // Guide MCP diagnostic endpoint protected by auth middleware
    let guide_mcp_authenticated = Router::new()
        .route("/api/system/guide-mcp", get(guide_mcp_status))
        .with_state(services.guide_mcp_config.clone())
        .route_layer(from_fn_with_state(auth_mw_state, auth_middleware));

    // Office proxy routes — exempt from auth (serve iframe content)
    let office_proxy = office_proxy_routes(states.office);
    let public_assets = asset_routes(AssetRouterState::default());

    // WebSocket upgrade route — exempt from CSRF (no cookie-based
    // double-submit) but still gets security response headers.
    let ws_routes = Router::new().route("/ws", get(ws_upgrade_handler)).with_state(ws_state);
    tracing::info!(elapsed_ms = boot.elapsed().as_millis(), "startup: route groups built");

    let router = Router::new()
        .route("/health", get(health_check))
        .merge(workbench_public)
        .merge(auth_routes(auth_state))
        .merge(system_authenticated)
        .merge(conversation_authenticated)
        .merge(conversation_ops_authenticated)
        .merge(remote_agent_authenticated)
        .merge(agent_authenticated)
        .merge(connection_test_authenticated)
        .merge(file_authenticated)
        .merge(mcp_authenticated)
        .merge(extension_authenticated)
        .merge(hub_authenticated)
        .merge(skill_authenticated)
        .merge(channel_authenticated)
        .merge(team_authenticated)
        .merge(cron_authenticated)
        .merge(office_authenticated)
        .merge(shell_authenticated)
        .merge(assistant_authenticated)
        .merge(workbench_state)
        .merge(guide_mcp_authenticated);

    // Conditionally merge WeChat login SSE route (feature-gated)
    #[cfg(feature = "weixin")]
    let router = router.merge(weixin_login_authenticated);

    let router = router
        .merge(ws_routes)
        .merge(office_proxy)
        .merge(public_assets)
        .layer(middleware::from_fn(security_headers_middleware));

    // Raise the default request body limit from axum's 2MB default to
    // `BODY_LIMIT` (10MB). Routes that need a larger cap (e.g. `/api/fs/upload`)
    // disable this default and install their own `RequestBodyLimitLayer`.
    let router = router.layer(DefaultBodyLimit::max(aionui_common::constants::BODY_LIMIT));
    let router = router.layer(middleware::from_fn(normalize_boundary_error_response));
    let router = router.layer(middleware::from_fn(normalize_json_api_response));

    let router = with_access_log(router);
    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: route tree build with states completed"
    );

    if services.local {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
                Method::OPTIONS,
            ])
            .allow_headers(Any);
        router.layer(cors)
    } else {
        router
    }
}

async fn normalize_json_api_response(request: Request, next: Next) -> Response {
    let is_api_request = request.uri().path().starts_with("/api/");
    let response = next.run(request).await;
    if !is_api_request || !response_has_json_content_type(&response) {
        return response;
    }

    let status = response.status();
    if response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > JSON_API_NORMALIZATION_MAX_BYTES)
    {
        tracing::warn!(
            status = status.as_u16(),
            max_bytes = JSON_API_NORMALIZATION_MAX_BYTES,
            "skipping JSON envelope normalization for oversized response"
        );
        return response;
    }

    let (mut parts, body) = response.into_parts();
    let bytes = match to_bytes(body, JSON_API_NORMALIZATION_MAX_BYTES).await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(error = %error, "failed to read JSON response body for envelope normalization");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "Failed to normalize response body",
                    i32::from(StatusCode::INTERNAL_SERVER_ERROR.as_u16()),
                )),
            )
                .into_response();
        }
    };

    if bytes.is_empty() {
        return Response::from_parts(parts, Body::from(bytes));
    }

    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return Response::from_parts(parts, Body::from(bytes)),
    };

    if is_standard_api_envelope(&value) {
        return Response::from_parts(parts, Body::from(bytes));
    }

    let normalized = normalize_legacy_json_value(status, value);
    let normalized_bytes = match serde_json::to_vec(&normalized) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(error = %error, "failed to serialize normalized JSON response body");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "Failed to serialize response body",
                    i32::from(StatusCode::INTERNAL_SERVER_ERROR.as_u16()),
                )),
            )
                .into_response();
        }
    };

    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(normalized_bytes))
}

fn is_standard_api_envelope(value: &Value) -> bool {
    value.as_object().is_some_and(|object| {
        object.contains_key("code")
            && object.contains_key("message")
            && object.contains_key("data")
            && object.contains_key("trace_id")
    })
}

fn normalize_legacy_json_value(status: StatusCode, value: Value) -> Value {
    match value {
        Value::Object(mut object) => normalize_legacy_json_object(status, &mut object),
        other => json!({
            "code": status_to_envelope_code(status),
            "message": status_to_envelope_message(status),
            "data": if status.is_success() { other } else { Value::Null },
            "trace_id": Value::Null,
        }),
    }
}

fn normalize_legacy_json_object(status: StatusCode, object: &mut Map<String, Value>) -> Value {
    let legacy_success = object.remove("success").and_then(|value| value.as_bool());
    let message = object
        .remove("message")
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .or_else(|| {
            object
                .remove("error")
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
        });
    let legacy_code = object.remove("code");

    let ok = legacy_success.unwrap_or_else(|| status.is_success());
    let code = if ok && status.is_success() {
        0
    } else {
        legacy_code
            .as_ref()
            .and_then(Value::as_i64)
            .or_else(|| {
                legacy_code
                    .as_ref()
                    .and_then(Value::as_u64)
                    .and_then(|value| i64::try_from(value).ok())
            })
            .unwrap_or_else(|| i64::from(status.as_u16()))
    };

    let data = if let Some(data) = object.remove("data") {
        data
    } else if object.is_empty() {
        Value::Null
    } else {
        Value::Object(std::mem::take(object))
    };

    let data = if ok {
        data
    } else {
        let mut details = match data {
            Value::Object(map) => map,
            Value::Null => Map::new(),
            other => {
                let mut map = Map::new();
                map.insert("details".to_owned(), other);
                map
            }
        };
        if let Some(code) = legacy_code
            && !code.is_null()
        {
            details.insert("error_code".to_owned(), code);
        }
        if details.is_empty() {
            Value::Null
        } else {
            Value::Object(details)
        }
    };

    json!({
        "code": code,
        "message": message.unwrap_or_else(|| status_to_envelope_message(status)),
        "data": data,
        "trace_id": Value::Null,
    })
}

fn status_to_envelope_code(status: StatusCode) -> i64 {
    if status.is_success() {
        0
    } else {
        i64::from(status.as_u16())
    }
}

fn status_to_envelope_message(status: StatusCode) -> String {
    if status.is_success() {
        "ok".to_owned()
    } else {
        status.canonical_reason().unwrap_or("Request failed").to_owned()
    }
}

async fn normalize_boundary_error_response(request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    if response.status().is_success() || response_has_json_content_type(&response) {
        return response;
    }

    let status = response.status();
    let Some(error) = boundary_error_for_status(status) else {
        return response;
    };

    let original_headers = response.headers().clone();
    let mut normalized = (status, Json(ErrorResponse::new(error, i32::from(status.as_u16())))).into_response();
    for (name, value) in original_headers.iter() {
        if *name != header::CONTENT_TYPE && *name != header::CONTENT_LENGTH {
            normalized.headers_mut().insert(name, value.clone());
        }
    }
    normalized
}

fn response_has_json_content_type(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|content_type| content_type.starts_with("application/json"))
}

fn boundary_error_for_status(status: StatusCode) -> Option<&'static str> {
    match status {
        StatusCode::BAD_REQUEST => Some("Bad request."),
        StatusCode::UNAUTHORIZED => Some("Unauthorized."),
        StatusCode::FORBIDDEN => Some("Forbidden."),
        StatusCode::NOT_FOUND => Some("Route not found."),
        StatusCode::METHOD_NOT_ALLOWED => Some("Method not allowed."),
        StatusCode::CONFLICT => Some("Conflict."),
        StatusCode::GONE => Some("Gone."),
        StatusCode::PAYLOAD_TOO_LARGE => Some("Request body is too large."),
        StatusCode::UNSUPPORTED_MEDIA_TYPE => Some("Unsupported media type."),
        StatusCode::UNPROCESSABLE_ENTITY => Some("Unprocessable entity."),
        StatusCode::TOO_MANY_REQUESTS => Some("Rate limited"),
        StatusCode::INTERNAL_SERVER_ERROR => Some("Internal server error."),
        StatusCode::BAD_GATEWAY => Some("Upstream service unavailable."),
        StatusCode::GATEWAY_TIMEOUT => Some("Request timed out."),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::{Json, Router, middleware};
    use tower::ServiceExt;

    use super::{boundary_error_for_status, normalize_json_api_response};

    async fn read_json(response: axum::response::Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should be readable");
        serde_json::from_slice(&bytes).expect("response should be JSON")
    }

    #[test]
    fn boundary_error_for_status_covers_common_fallback_statuses() {
        let cases = [
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::NOT_FOUND,
            StatusCode::METHOD_NOT_ALLOWED,
            StatusCode::CONFLICT,
            StatusCode::GONE,
            StatusCode::PAYLOAD_TOO_LARGE,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::GATEWAY_TIMEOUT,
        ];

        for status in cases {
            let message = boundary_error_for_status(status).expect("status should be normalized");
            assert!(!message.is_empty());
        }
    }

    #[tokio::test]
    async fn json_normalizer_wraps_legacy_success_response() {
        let app = Router::new()
            .route(
                "/api/legacy",
                get(|| async { Json(serde_json::json!({ "success": true, "token": "abc" })) }),
            )
            .layer(middleware::from_fn(normalize_json_api_response));

        let response = app
            .oneshot(Request::builder().uri("/api/legacy").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let json = read_json(response).await;

        assert_eq!(json["code"], 0);
        assert_eq!(json["message"], "ok");
        assert_eq!(json["data"]["token"], "abc");
        assert_eq!(json["trace_id"], serde_json::Value::Null);
        assert!(json.get("success").is_none());
    }

    #[tokio::test]
    async fn json_normalizer_leaves_standard_envelope_unchanged() {
        let app = Router::new()
            .route(
                "/api/current",
                get(|| async {
                    Json(serde_json::json!({
                        "code": 0,
                        "message": "ok",
                        "data": {"value": 1},
                        "trace_id": null
                    }))
                }),
            )
            .layer(middleware::from_fn(normalize_json_api_response));

        let response = app
            .oneshot(Request::builder().uri("/api/current").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let json = read_json(response).await;

        assert_eq!(
            json,
            serde_json::json!({
                "code": 0,
                "message": "ok",
                "data": {"value": 1},
                "trace_id": null
            })
        );
    }

    #[tokio::test]
    async fn json_normalizer_wraps_raw_json_array() {
        let app = Router::new()
            .route("/api/items", get(|| async { Json(serde_json::json!([1, 2])) }))
            .layer(middleware::from_fn(normalize_json_api_response));

        let response = app
            .oneshot(Request::builder().uri("/api/items").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let json = read_json(response).await;

        assert_eq!(json["code"], 0);
        assert_eq!(json["message"], "ok");
        assert_eq!(json["data"], serde_json::json!([1, 2]));
        assert_eq!(json["trace_id"], serde_json::Value::Null);
    }
}
