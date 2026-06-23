//! End-to-end integration tests for the complete authentication flow.
//!
//! These tests exercise the full application stack (security headers and
//! auth routes) via `aionui_app::create_router`, covering test-plan items
//! T12 (security middleware), T13 (token extraction), T14 (initial bootstrap).

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

use aionui_app::{AppConfig, AppServices};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

async fn build_app() -> (axum::Router, AppServices) {
    let db = aionui_db::init_database_memory().await.unwrap();
    let services = AppServices::from_config(db, &AppConfig::default()).await.unwrap();
    let router = aionui_app::create_router(&services).await.expect("build router");
    (router, services)
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// Extract the CSRF token from a Set-Cookie header.
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

/// Extract the session token from a Set-Cookie header.
fn extract_session_token(resp: &axum::response::Response) -> Option<String> {
    resp.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|s| s.starts_with("aionui-session="))
        .and_then(|s| {
            let value = s.strip_prefix("aionui-session=")?.split(';').next()?.to_owned();
            if value.is_empty() { None } else { Some(value) }
        })
}

fn get_request(uri: &str) -> Request<Body> {
    Request::builder().method("GET").uri(uri).body(Body::empty()).unwrap()
}

fn get_with_token(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

fn get_with_cookie(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("cookie", format!("aionui-session={token}"))
        .body(Body::empty())
        .unwrap()
}

fn post_json_login(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

fn post_json_with_csrf(uri: &str, body: &str, token: &str, csrf: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("x-csrf-token", csrf)
        .header("cookie", format!("aionui-csrf-token={csrf}"))
        .body(Body::from(body.to_owned()))
        .unwrap()
}

/// Set up a user and login, returning (session_token, legacy_empty_csrf_token).
///
/// Seeded `system_default_user` already owns `username = "admin"` with an empty
/// hash; if the test uses that name, overwrite the seed row in place. Other
/// usernames use the normal create_user path.
async fn setup_and_login(
    app: &mut axum::Router,
    services: &AppServices,
    username: &str,
    password: &str,
) -> (String, String) {
    // Create user
    let hash = aionui_auth::hash_password(password).unwrap();
    if username == "admin" {
        services
            .user_repo
            .set_system_user_credentials(username, &hash)
            .await
            .unwrap();
    } else {
        services.user_repo.create_user(username, &hash).await.unwrap();
    }

    let body = format!(r#"{{"username":"{username}","password":"{password}"}}"#);
    let resp = app.clone().oneshot(post_json_login("/login", &body)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "login should succeed");

    let json = body_json(resp).await;
    let token = json["data"]["token"].as_str().unwrap().to_owned();

    (token, String::new())
}

// ===========================================================================
// T12. Security Middleware
// ===========================================================================

#[tokio::test]
async fn t12_1_security_headers_on_all_responses() {
    let (app, _services) = build_app().await;

    let resp = app.oneshot(get_request("/health")).await.unwrap();

    assert_eq!(resp.headers().get("x-frame-options").unwrap(), "DENY");
    assert_eq!(resp.headers().get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(resp.headers().get("x-xss-protection").unwrap(), "1; mode=block");
    assert_eq!(
        resp.headers().get("referrer-policy").unwrap(),
        "strict-origin-when-cross-origin"
    );
}

#[tokio::test]
async fn t12_1_security_headers_on_error_responses() {
    let (app, _services) = build_app().await;

    // 404 response should still have security headers
    let resp = app.oneshot(get_request("/nonexistent")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(resp.headers().get("x-frame-options").unwrap(), "DENY");
}

#[tokio::test]
async fn t12_2_bearer_post_does_not_require_csrf() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = Request::builder()
        .method("POST")
        .uri("/logout")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn t12_2_state_changing_post_without_auth_is_unauthorized() {
    let (app, _services) = build_app().await;

    let req = Request::builder()
        .method("POST")
        .uri("/logout")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn t12_2_login_does_not_require_csrf() {
    let (app, _services) = build_app().await;

    let req = post_json_login("/login", r#"{"username":"x","password":"y"}"#);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let req = post_json_login("/api/auth/qr-login", r#"{"qr_token":"fake"}"#);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn t12_3_session_cookie_attributes() {
    let (app, services) = build_app().await;
    let hash = aionui_auth::hash_password("StrongP@ss1").unwrap();
    // system_default_user is seeded with username='admin'; overwrite its empty
    // password in place instead of creating a duplicate.
    services
        .user_repo
        .set_system_user_credentials("admin", &hash)
        .await
        .unwrap();

    let req = post_json_login("/login", r#"{"username":"admin","password":"StrongP@ss1"}"#);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let set_cookie = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|s| s.starts_with("aionui-session="))
        .expect("session cookie should be set");

    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite="));
    assert!(set_cookie.contains("Max-Age="));
    // Max-Age should be 30 days
    let expected_max_age = format!("Max-Age={}", 30 * 24 * 60 * 60);
    assert!(set_cookie.contains(&expected_max_age));
}

// ===========================================================================
// T13. Token Extraction Strategy
// ===========================================================================

#[tokio::test]
async fn t13_1_authorization_header_takes_priority() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Both header and cookie present; header should be used
    let req = Request::builder()
        .method("GET")
        .uri("/api/auth/user")
        .header("authorization", format!("Bearer {token}"))
        .header("cookie", "aionui-session=invalid_token")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["user"]["username"], "admin");
}

#[tokio::test]
async fn t13_2_cookie_fallback() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Only cookie, no Authorization header
    let req = get_with_cookie("/api/auth/user", &token);
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["user"]["username"], "admin");
}

#[tokio::test]
async fn t13_3_no_token_fails() {
    let (app, _services) = build_app().await;

    let req = get_request("/api/auth/user");
    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let json = body_json(resp).await;
    assert_eq!(json["code"], 401);
}

// ===========================================================================
// T14. Initial Bootstrap Flow
// ===========================================================================

#[tokio::test]
async fn t14_1_seeded_system_does_not_need_setup() {
    let (app, _services) = build_app().await;

    let resp = app.oneshot(get_request("/api/auth/status")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["code"], 0);
    assert_eq!(json["data"]["needs_setup"], false);
}

#[tokio::test]
async fn t14_2_seeded_system_credentials_then_login() {
    let (app, services) = build_app().await;

    // The memory test database seeds a system user, so setup is not required.
    let resp = app.clone().oneshot(get_request("/api/auth/status")).await.unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"]["needs_setup"], false);

    // Set system user credentials (simulating initial setup)
    let hash = aionui_auth::hash_password("Admin@Pass1").unwrap();
    services
        .user_repo
        .set_system_user_credentials("admin", &hash)
        .await
        .unwrap();

    // Now needsSetup=false
    let resp = app.clone().oneshot(get_request("/api/auth/status")).await.unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"]["needs_setup"], false);

    // Login with new credentials
    let req = post_json_login("/login", r#"{"username":"admin","password":"Admin@Pass1"}"#);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["code"], 0);
    assert_eq!(json["data"]["user"]["username"], "admin");

    // Authenticated status check
    let token = json["data"]["token"].as_str().unwrap();
    let req = get_with_token("/api/auth/status", token);
    let resp = app.oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"]["is_authenticated"], true);
    assert_eq!(json["data"]["needs_setup"], false);
}

// ===========================================================================
// Full E2E Flow: setup → login → get user → change password → logout
// ===========================================================================

#[tokio::test]
async fn full_auth_flow_e2e() {
    let (app, services) = build_app().await;

    // 1. Check initial status
    let resp = app.clone().oneshot(get_request("/api/auth/status")).await.unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"]["needs_setup"], false);

    // 2. Setup user
    let hash = aionui_auth::hash_password("Initial@Pass1").unwrap();
    services
        .user_repo
        .set_system_user_credentials("admin", &hash)
        .await
        .unwrap();

    // 3. Login
    let req = post_json_login("/login", r#"{"username":"admin","password":"Initial@Pass1"}"#);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let session_token = extract_session_token(&resp).expect("session cookie set");
    let json = body_json(resp).await;
    let token = json["data"]["token"].as_str().unwrap().to_owned();

    // Verify session token matches response body token
    assert_eq!(session_token, token);

    // 4. Get current user
    let req = get_with_token("/api/auth/user", &token);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["user"]["username"], "admin");

    // 5. Change password
    let req = post_json_with_csrf(
        "/api/auth/change-password",
        r#"{"current_password":"Initial@Pass1","new_password":"Updated@Pass2"}"#,
        &token,
        "",
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 6. Old token invalidated after password change
    let req = get_with_token("/api/auth/user", &token);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let json = body_json(resp).await;
    assert_eq!(json["code"], 401);

    // 7. Login with new password
    let req = post_json_login("/login", r#"{"username":"admin","password":"Updated@Pass2"}"#);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let new_token = json["data"]["token"].as_str().unwrap().to_owned();

    // 8. Logout
    let req = post_json_with_csrf("/logout", "", &new_token, "");
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 9. Token invalid after logout
    let req = get_with_token("/api/auth/user", &new_token);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let json = body_json(resp).await;
    assert_eq!(json["code"], 401);
}

// ===========================================================================
// CSRF cookie is not part of the SaaS frontend contract
// ===========================================================================

#[tokio::test]
async fn csrf_cookie_not_set_on_first_get() {
    let (app, _services) = build_app().await;

    let resp = app.oneshot(get_request("/health")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let csrf = extract_csrf_token(&resp);
    assert!(csrf.is_none(), "CSRF cookie should not be set on first request");
}
