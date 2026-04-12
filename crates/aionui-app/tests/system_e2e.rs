//! End-to-end integration tests for system routes through the full app stack.
//!
//! These tests exercise the complete middleware chain (security headers, CSRF,
//! auth) for all system endpoints: settings, client preferences, providers,
//! model fetch, protocol detection, system info, and version check.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;
use wiremock::matchers::{header as match_header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aionui_app::{AppServices, build_system_state, create_router, create_router_with_system_state};
use aionui_system::VersionCheckService;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

async fn build_app() -> (axum::Router, AppServices) {
    let db = aionui_db::init_database_memory().await.unwrap();
    let services = AppServices::from_database(db).await.unwrap();
    let router = create_router(&services);
    (router, services)
}

async fn build_app_with_mock_version(
    current_version: &str,
    mock_server: &MockServer,
) -> (axum::Router, AppServices) {
    let db = aionui_db::init_database_memory().await.unwrap();
    let services = AppServices::from_database(db).await.unwrap();
    let mut system_state = build_system_state(&services);
    system_state.version_check_service = VersionCheckService::with_api_base(
        reqwest::Client::new(),
        current_version.to_owned(),
        mock_server.uri(),
    );
    let router = create_router_with_system_state(&services, system_state);
    (router, services)
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn extract_csrf_token(resp: &axum::response::Response) -> Option<String> {
    resp.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|s| s.starts_with("aionui-csrf-token="))
        .map(|s| {
            s.strip_prefix("aionui-csrf-token=")
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .to_owned()
        })
}

fn get_request(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn get_with_token(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

fn json_with_token(
    method_str: &str,
    uri: &str,
    body: serde_json::Value,
    token: &str,
    csrf: &str,
) -> Request<Body> {
    Request::builder()
        .method(method_str)
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("x-csrf-token", csrf)
        .header("cookie", format!("aionui-csrf-token={csrf}"))
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn delete_with_token(uri: &str, token: &str, csrf: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("x-csrf-token", csrf)
        .header("cookie", format!("aionui-csrf-token={csrf}"))
        .body(Body::empty())
        .unwrap()
}

/// Set up a user and login, returning (session_token, csrf_token).
async fn setup_and_login(
    app: &mut axum::Router,
    services: &AppServices,
    username: &str,
    password: &str,
) -> (String, String) {
    let hash = aionui_auth::hash_password(password).unwrap();
    services.user_repo.create_user(username, &hash).await.unwrap();

    let resp = app.clone().oneshot(get_request("/api/auth/status")).await.unwrap();
    let csrf = extract_csrf_token(&resp).expect("CSRF cookie should be set");

    let body = format!(r#"{{"username":"{username}","password":"{password}"}}"#);
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "login should succeed");

    let json = body_json(resp).await;
    let token = json["token"].as_str().unwrap().to_owned();

    (token, csrf)
}

// ===========================================================================
// Auth protection — all system endpoints return 403 without auth
// ===========================================================================

#[tokio::test]
async fn auth_required_get_settings() {
    let (app, _) = build_app().await;
    let resp = app.oneshot(get_request("/api/settings")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_patch_settings() {
    let (app, _) = build_app().await;
    let req = Request::builder()
        .method("PATCH")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"language":"en-US"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_get_client_prefs() {
    let (app, _) = build_app().await;
    let resp = app.oneshot(get_request("/api/settings/client")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_put_client_prefs() {
    let (app, _) = build_app().await;
    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings/client")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"key":"value"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_get_providers() {
    let (app, _) = build_app().await;
    let resp = app.oneshot(get_request("/api/providers")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_post_providers() {
    let (app, _) = build_app().await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/providers")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"platform":"openai","name":"Test","baseUrl":"https://api.openai.com","apiKey":"sk-test"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_delete_provider() {
    let (app, _) = build_app().await;
    let req = Request::builder()
        .method("DELETE")
        .uri("/api/providers/some-id")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_system_info() {
    let (app, _) = build_app().await;
    let resp = app.oneshot(get_request("/api/system/info")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_check_update() {
    let (app, _) = build_app().await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/system/check-update")
        .header("content-type", "application/json")
        .body(Body::from(r#"{}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_detect_protocol() {
    let (app, _) = build_app().await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/providers/detect-protocol")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"baseUrl":"https://api.example.com","apiKey":"sk-test"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_fetch_models() {
    let (app, _) = build_app().await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/providers/some-id/models")
        .header("content-type", "application/json")
        .body(Body::from(r#"{}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ===========================================================================
// Settings CRUD with auth
// ===========================================================================

#[tokio::test]
async fn settings_get_default_values_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let resp = app
        .oneshot(get_with_token("/api/settings", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["language"], "en-US");
    assert_eq!(json["data"]["notificationEnabled"], true);
}

#[tokio::test]
async fn settings_patch_and_get_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // PATCH
    let req = json_with_token(
        "PATCH",
        "/api/settings",
        json!({"language": "zh-CN", "notificationEnabled": false}),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["language"], "zh-CN");
    assert_eq!(json["data"]["notificationEnabled"], false);

    // GET — should reflect changes
    let resp = app
        .oneshot(get_with_token("/api/settings", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"]["language"], "zh-CN");
    assert_eq!(json["data"]["notificationEnabled"], false);
}

#[tokio::test]
async fn settings_invalid_language_rejected_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "PATCH",
        "/api/settings",
        json!({"language": "invalid-lang"}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ===========================================================================
// Client Preferences CRUD with auth
// ===========================================================================

#[tokio::test]
async fn client_prefs_empty_then_write_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // GET — empty
    let resp = app
        .clone()
        .oneshot(get_with_token("/api/settings/client", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"], json!({}));

    // PUT — write values
    let req = json_with_token(
        "PUT",
        "/api/settings/client",
        json!({"theme": "dark", "pet.size": 360, "system.closeToTray": true}),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // GET — should reflect changes
    let resp = app
        .clone()
        .oneshot(get_with_token("/api/settings/client", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"]["theme"], "dark");
    assert_eq!(json["data"]["pet.size"], 360);
    assert_eq!(json["data"]["system.closeToTray"], true);

    // PUT — delete via null
    let req = json_with_token(
        "PUT",
        "/api/settings/client",
        json!({"theme": null}),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // GET — theme should be gone
    let resp = app
        .oneshot(get_with_token("/api/settings/client", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert!(json["data"].get("theme").is_none());
    assert_eq!(json["data"]["pet.size"], 360);
}

#[tokio::test]
async fn client_prefs_key_filter_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "PUT",
        "/api/settings/client",
        json!({"a": 1, "b": 2, "c": 3}),
        &token,
        &csrf,
    );
    app.clone().oneshot(req).await.unwrap();

    let resp = app
        .oneshot(get_with_token("/api/settings/client?keys=a,c", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    let data = json["data"].as_object().unwrap();
    assert_eq!(data.len(), 2);
    assert_eq!(data["a"], 1);
    assert_eq!(data["c"], 3);
}

// ===========================================================================
// Provider CRUD with auth
// ===========================================================================

#[tokio::test]
async fn provider_full_crud_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // 1. List — empty
    let resp = app
        .clone()
        .oneshot(get_with_token("/api/providers", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"], json!([]));

    // 2. Create
    let req = json_with_token(
        "POST",
        "/api/providers",
        json!({
            "platform": "anthropic",
            "name": "Anthropic",
            "baseUrl": "https://api.anthropic.com",
            "apiKey": "sk-ant-api03-test1234"
        }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    let id = json["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(json["data"]["platform"], "anthropic");
    assert_eq!(json["data"]["name"], "Anthropic");
    // API key should be masked
    let api_key = json["data"]["apiKey"].as_str().unwrap();
    assert!(api_key.contains("***"), "API key should be masked");
    assert!(api_key.ends_with("1234"), "mask should show last 4 chars");

    // 3. List — should contain one
    let resp = app
        .clone()
        .oneshot(get_with_token("/api/providers", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"].as_array().unwrap().len(), 1);

    // 4. Update
    let req = json_with_token(
        "PUT",
        &format!("/api/providers/{id}"),
        json!({"name": "Updated Name", "enabled": false}),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["name"], "Updated Name");
    assert!(!json["data"]["enabled"].as_bool().unwrap());

    // 5. Delete
    let resp = app
        .clone()
        .oneshot(delete_with_token(&format!("/api/providers/{id}"), &token, &csrf))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 6. Verify deleted
    let resp = app
        .oneshot(get_with_token("/api/providers", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"], json!([]));
}

#[tokio::test]
async fn provider_create_validation_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Missing platform
    let req = json_with_token(
        "POST",
        "/api/providers",
        json!({"name": "Test", "baseUrl": "https://api.example.com", "apiKey": "sk-test"}),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Invalid URL
    let req = json_with_token(
        "POST",
        "/api/providers",
        json!({"platform": "openai", "name": "Test", "baseUrl": "not-a-url", "apiKey": "sk-test"}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn provider_update_nonexistent_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "PUT",
        "/api/providers/nonexistent",
        json!({"name": "X"}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn provider_delete_nonexistent_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let resp = app
        .oneshot(delete_with_token("/api/providers/nonexistent", &token, &csrf))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ===========================================================================
// System info with auth
// ===========================================================================

#[tokio::test]
async fn system_info_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let resp = app
        .oneshot(get_with_token("/api/system/info", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["success"], true);

    let data = &json["data"];
    assert!(data["cacheDir"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(data["workDir"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(data["logDir"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(["darwin", "win32", "linux"].contains(&data["platform"].as_str().unwrap()));
    assert!(["x64", "arm64"].contains(&data["arch"].as_str().unwrap()));
}

// ===========================================================================
// Version check with auth + wiremock
// ===========================================================================

fn make_github_release(
    tag: &str,
    draft: bool,
    prerelease: bool,
    assets: Vec<serde_json::Value>,
) -> serde_json::Value {
    json!({
        "tag_name": tag,
        "name": format!("Release {tag}"),
        "body": "Release notes",
        "html_url": format!("https://github.com/iOfficeAI/AionUi/releases/tag/{tag}"),
        "published_at": "2026-04-01T00:00:00Z",
        "prerelease": prerelease,
        "draft": draft,
        "assets": assets,
    })
}

fn make_github_asset(name: &str, size: u64) -> serde_json::Value {
    json!({
        "name": name,
        "browser_download_url": format!("https://github.com/download/{name}"),
        "size": size,
        "content_type": "application/octet-stream",
    })
}

#[tokio::test]
async fn version_check_has_update_with_auth() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/iOfficeAI/AionUi/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            make_github_release("v2.0.0", false, false, vec![
                make_github_asset("app-2.0.0-darwin-arm64.dmg", 80_000_000),
            ]),
        ])))
        .mount(&mock_server)
        .await;

    let (mut app, services) = build_app_with_mock_version("1.0.0", &mock_server).await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/system/check-update",
        json!({}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["updateAvailable"], true);
    assert_eq!(json["data"]["latest"]["version"], "2.0.0");
}

#[tokio::test]
async fn version_check_no_update_with_auth() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/iOfficeAI/AionUi/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            make_github_release("v1.0.0", false, false, vec![]),
        ])))
        .mount(&mock_server)
        .await;

    let (mut app, services) = build_app_with_mock_version("1.0.0", &mock_server).await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/system/check-update",
        json!({}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["updateAvailable"], false);
}

#[tokio::test]
async fn version_check_github_error_with_auth() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/iOfficeAI/AionUi/releases"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Error"))
        .mount(&mock_server)
        .await;

    let (mut app, services) = build_app_with_mock_version("1.0.0", &mock_server).await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/system/check-update",
        json!({}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
}

// ===========================================================================
// Model fetch with auth + wiremock
// ===========================================================================

#[tokio::test]
async fn model_fetch_openai_with_auth() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": "gpt-4o"}, {"id": "gpt-4o-mini"}]
        })))
        .mount(&mock_server)
        .await;

    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Create a provider pointing to mock server
    let req = json_with_token(
        "POST",
        "/api/providers",
        json!({
            "platform": "openai",
            "name": "OpenAI Mock",
            "baseUrl": mock_server.uri(),
            "apiKey": "test-api-key"
        }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    let id = json["data"]["id"].as_str().unwrap().to_string();

    // Fetch models
    let req = json_with_token(
        "POST",
        &format!("/api/providers/{id}/models"),
        json!({"tryFix": false}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let models = json["data"]["models"].as_array().unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0], "gpt-4o");
}

#[tokio::test]
async fn model_fetch_nonexistent_provider_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/providers/nonexistent/models",
        json!({}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ===========================================================================
// Protocol detection with auth + wiremock
// ===========================================================================

#[tokio::test]
async fn protocol_detect_openai_with_auth() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .and(match_header("Authorization", "Bearer sk-test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": "gpt-4"}, {"id": "gpt-3.5-turbo"}]
        })))
        .mount(&mock_server)
        .await;

    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/providers/detect-protocol",
        json!({
            "baseUrl": mock_server.uri(),
            "apiKey": "sk-test-key"
        }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["protocol"], "openai");
    assert!(json["data"]["confidence"].as_u64().unwrap() > 0);
    let models = json["data"]["models"].as_array().unwrap();
    assert!(!models.is_empty());
}

#[tokio::test]
async fn protocol_detect_all_fail_with_auth() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock_server)
        .await;

    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/providers/detect-protocol",
        json!({
            "baseUrl": mock_server.uri(),
            "apiKey": "sk-unknown"
        }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["protocol"], "unknown");
    assert_eq!(json["data"]["confidence"], 0);
}

#[tokio::test]
async fn protocol_detect_validation_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Missing baseUrl
    let req = json_with_token(
        "POST",
        "/api/providers/detect-protocol",
        json!({"apiKey": "sk-test"}),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Missing apiKey
    let req = json_with_token(
        "POST",
        "/api/providers/detect-protocol",
        json!({"baseUrl": "https://api.example.com"}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn protocol_detect_switch_platform_suggestion_with_auth() {
    let mock_server = MockServer::start().await;

    // Only OpenAI endpoint works
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": "gpt-4"}]
        })))
        .mount(&mock_server)
        .await;

    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Preferred is anthropic but detected is openai → switch_platform
    let req = json_with_token(
        "POST",
        "/api/providers/detect-protocol",
        json!({
            "baseUrl": mock_server.uri(),
            "apiKey": "sk-test",
            "preferredProtocol": "anthropic"
        }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["protocol"], "openai");
    assert_eq!(json["data"]["suggestion"]["type"], "switch_platform");
}

// ===========================================================================
// Full authenticated flow — settings + providers round-trip
// ===========================================================================

#[tokio::test]
async fn full_system_flow_e2e() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // 1. Get default settings
    let resp = app
        .clone()
        .oneshot(get_with_token("/api/settings", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["language"], "en-US");

    // 2. Update language
    let req = json_with_token(
        "PATCH",
        "/api/settings",
        json!({"language": "zh-CN"}),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["language"], "zh-CN");

    // 3. Write client preferences
    let req = json_with_token(
        "PUT",
        "/api/settings/client",
        json!({"theme": "dark", "sidebar.width": 280}),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 4. Verify preferences
    let resp = app
        .clone()
        .oneshot(get_with_token("/api/settings/client", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"]["theme"], "dark");
    assert_eq!(json["data"]["sidebar.width"], 280);

    // 5. Get system info
    let resp = app
        .clone()
        .oneshot(get_with_token("/api/system/info", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["data"]["platform"].as_str().is_some());

    // 6. Create provider
    let req = json_with_token(
        "POST",
        "/api/providers",
        json!({
            "platform": "openai",
            "name": "OpenAI",
            "baseUrl": "https://api.openai.com",
            "apiKey": "sk-proj-test-key-1234"
        }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    let provider_id = json["data"]["id"].as_str().unwrap().to_string();
    assert!(json["data"]["apiKey"].as_str().unwrap().contains("***"));

    // 7. List providers
    let resp = app
        .clone()
        .oneshot(get_with_token("/api/providers", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"].as_array().unwrap().len(), 1);

    // 8. Delete provider
    let resp = app
        .oneshot(delete_with_token(
            &format!("/api/providers/{provider_id}"),
            &token,
            &csrf,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
