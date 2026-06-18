use std::sync::Arc;

use aionui_api_types::WebSocketMessage;
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tracing::{debug, info};

use crate::manager::{TokenValidator, WebSocketManager};
use crate::router::MessageRouter;
use crate::types::{ConnectionId, PER_CONNECTION_BUFFER, RealtimeError, WebSocketCloseCode, WsOutbound};

/// Extracts a JWT token from WebSocket upgrade request headers.
///
/// Injected by `aionui-app` — wraps `aionui_auth::extract_token_from_ws_headers`
/// so that `aionui-realtime` does not depend on `aionui-auth` directly.
pub type TokenExtractor = Arc<dyn Fn(&HeaderMap) -> Option<String> + Send + Sync>;

/// Shared state required by the WebSocket upgrade handler.
#[derive(Clone)]
pub struct WsHandlerState {
    pub manager: Arc<WebSocketManager>,
    pub router: Arc<dyn MessageRouter>,
    pub token_validator: TokenValidator,
    pub token_extractor: TokenExtractor,
}

/// Axum handler for HTTP → WebSocket upgrade.
///
/// Extracts a JWT token from the request headers, validates it,
/// and upgrades the connection to WebSocket on success.
/// On authentication failure, rejects the upgrade with HTTP 401.
///
/// When the token is carried via `Sec-WebSocket-Protocol`, the server
/// echoes only the validated token-shaped protocol value back so the client
/// handshake succeeds without reflecting arbitrary client input.
pub async fn ws_upgrade_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    axum::extract::State(state): axum::extract::State<WsHandlerState>,
) -> Response {
    let Some(token) = (state.token_extractor)(&headers) else {
        return ws
            .on_upgrade(move |socket| async move {
                close_unauthorized(socket, RealtimeError::AuthMissing).await;
            })
            .into_response();
    };

    if !is_token_shaped_protocol_value(&token) || !(state.token_validator)(&token) {
        let ws = if let Some(protocol) = validated_response_protocol(&headers, &token) {
            ws.protocols([protocol])
        } else {
            ws
        };
        return ws
            .on_upgrade(move |socket| async move {
                close_unauthorized(socket, RealtimeError::AuthExpired).await;
            })
            .into_response();
    }

    let ws = if let Some(protocol) = validated_response_protocol(&headers, &token) {
        ws.protocols([protocol])
    } else {
        ws
    };

    ws.on_upgrade(move |socket| async move {
        handle_socket(socket, token, state).await;
    })
    .into_response()
}

/// Post-upgrade connection handler.
///
/// Validates the token, registers the client, spawns send/recv loops.
async fn handle_socket(socket: WebSocket, token: String, state: WsHandlerState) {
    if !(state.token_validator)(&token) {
        debug!("websocket token expired after upgrade");
        close_unauthorized(socket, RealtimeError::AuthExpired).await;
        return;
    }

    let (tx, rx) = mpsc::channel::<WsOutbound>(PER_CONNECTION_BUFFER);
    let conn_id = state.manager.add_client(token, tx);

    info!(%conn_id, "websocket connection established");

    let (ws_sender, ws_receiver) = socket.split();

    let send_handle = tokio::spawn(send_loop(conn_id, rx, ws_sender));
    recv_loop(conn_id, ws_receiver, &state).await;

    // Recv loop exited — client disconnected or errored.
    send_handle.abort();
    state.manager.remove_client(conn_id);
    info!(%conn_id, "websocket connection closed");
}

async fn close_unauthorized(mut socket: WebSocket, error: RealtimeError) {
    let text = serde_json::to_string(&error.into_event()).unwrap_or_else(|_| {
        r#"{"name":"realtime.error","data":{"code":"REALTIME_INTERNAL_ERROR","message":"Realtime boundary error.","recoverable":false,"details":{}}}"#.to_owned()
    });
    let _ = socket.send(Message::Text(text.into())).await;
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code: WebSocketCloseCode::PolicyViolation.as_u16(),
            reason: error.message().into(),
        })))
        .await;
}

fn validated_response_protocol(headers: &HeaderMap, token: &str) -> Option<String> {
    let protocol = first_websocket_protocol(headers)?;
    if protocol == token && is_token_shaped_protocol_value(protocol) {
        Some(protocol.to_owned())
    } else {
        None
    }
}

fn first_websocket_protocol(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("sec-websocket-protocol")
        .and_then(|value| value.to_str().ok())
        .and_then(|protocols| protocols.split(',').next())
        .map(str::trim)
        .filter(|protocol| !protocol.is_empty())
}

fn is_token_shaped_protocol_value(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(is_websocket_subprotocol_tchar)
}

fn is_websocket_subprotocol_tchar(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
        )
}

// -------------------------------------------------------------------
// Send loop
// -------------------------------------------------------------------

/// Reads `WsOutbound` from the per-connection channel and forwards
/// them to the WebSocket sink.
async fn send_loop(
    conn_id: ConnectionId,
    mut rx: mpsc::Receiver<WsOutbound>,
    mut sender: futures_util::stream::SplitSink<WebSocket, Message>,
) {
    while let Some(outbound) = rx.recv().await {
        let msg = match outbound {
            WsOutbound::Text(text) => Message::Text(text.into()),
            WsOutbound::Close(code, reason) => Message::Close(Some(CloseFrame {
                code: code.as_u16(),
                reason: reason.into(),
            })),
            WsOutbound::TextThenClose(text, code, reason) => {
                if sender.send(Message::Text(text.into())).await.is_err() {
                    debug!(%conn_id, "send loop: socket write failed, exiting");
                    break;
                }
                Message::Close(Some(CloseFrame {
                    code: code.as_u16(),
                    reason: reason.into(),
                }))
            }
        };
        if sender.send(msg).await.is_err() {
            debug!(%conn_id, "send loop: socket write failed, exiting");
            break;
        }
    }
}

// -------------------------------------------------------------------
// Receive loop
// -------------------------------------------------------------------

/// Reads messages from the WebSocket stream, parses JSON, routes.
async fn recv_loop(
    conn_id: ConnectionId,
    mut receiver: futures_util::stream::SplitStream<WebSocket>,
    state: &WsHandlerState,
) {
    while let Some(result) = receiver.next().await {
        let msg = match result {
            Ok(m) => m,
            Err(e) => {
                debug!(%conn_id, error = %e, "recv error, closing");
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                handle_text_message(conn_id, &text, state);
            }
            Message::Close(_) => {
                debug!(%conn_id, "received close frame");
                break;
            }
            // Ping/Pong at the WebSocket protocol level are handled
            // automatically by axum/tungstenite. Binary frames are ignored.
            _ => {}
        }
    }
}

/// Process a text message: parse JSON, dispatch to built-in or router.
fn handle_text_message(conn_id: ConnectionId, text: &str, state: &WsHandlerState) {
    let parsed: Result<WebSocketMessage<Value>, _> = serde_json::from_str(text);

    let msg = match parsed {
        Ok(m) => m,
        Err(_) => {
            send_error_response(state, conn_id);
            return;
        }
    };

    if msg.name.trim().is_empty() {
        send_error_response(state, conn_id);
        return;
    }

    match msg.name.as_str() {
        "pong" => {
            state.manager.update_last_ping(conn_id);
        }
        "subscribe-show-open" => {
            handle_subscribe_show_open(state, conn_id, msg.data);
        }
        name => {
            if !state.router.route(conn_id, name, msg.data) {
                send_realtime_error(state, conn_id, RealtimeError::UnsupportedMessage);
            }
        }
    }
}

/// Send an error response for invalid message format.
fn send_error_response(state: &WsHandlerState, conn_id: ConnectionId) {
    send_realtime_error(state, conn_id, RealtimeError::InvalidMessage);
}

fn send_realtime_error(state: &WsHandlerState, conn_id: ConnectionId, error: RealtimeError) {
    state.manager.send_to(conn_id, error.into_event());
}

/// Handle `subscribe-show-open`: reply with `show-open-request`.
///
/// The inbound `data` is the @office-ai/platform bridge envelope
/// `{ id, data: <user-params> }`. The renderer awaits a callback whose event
/// name embeds `id` (`subscribe.callback-show-open<id>`), so we must echo it
/// back; without it, the frontend's `useDirectorySelection` hook builds the
/// wrong callback name and the original `invoke()` Promise never resolves.
///
/// `isFileMode` is `true` when `properties` contains `openFile`
/// but NOT `openDirectory`.
fn handle_subscribe_show_open(state: &WsHandlerState, conn_id: ConnectionId, data: Value) {
    let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let inner = data.get("data").unwrap_or(&Value::Null);

    let properties = inner
        .get("properties")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let has_open_file = properties.iter().any(|v| v.as_str() == Some("openFile"));
    let has_open_directory = properties.iter().any(|v| v.as_str() == Some("openDirectory"));

    let is_file_mode = has_open_file && !has_open_directory;

    let response = WebSocketMessage::new(
        "show-open-request",
        json!({
            "id": id,
            "properties": properties,
            "isFileMode": is_file_mode,
        }),
    );

    state.manager.send_to(conn_id, response);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state(manager: Arc<WebSocketManager>) -> WsHandlerState {
        WsHandlerState {
            manager,
            router: Arc::new(crate::router::NoopMessageRouter),
            token_validator: Arc::new(|_| true),
            token_extractor: Arc::new(|_| None),
        }
    }

    fn assert_invalid_message_error(text: &str) {
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["name"], "realtime.error");
        assert_eq!(parsed["data"]["code"], "REALTIME_INVALID_MESSAGE");
        assert!(parsed["data"]["message"].is_string());
        assert_eq!(parsed["data"]["recoverable"], true);
        assert_eq!(
            parsed["data"]["details"]["expected"],
            r#"{ "name": "event-name", "data": {...} }"#
        );
    }

    fn assert_unsupported_message_error(text: &str) {
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["name"], "realtime.error");
        assert_eq!(parsed["data"]["code"], "REALTIME_UNSUPPORTED_MESSAGE");
        assert_eq!(parsed["data"]["recoverable"], true);
        assert!(parsed["data"]["message"].is_string());
        assert!(parsed["data"]["details"].is_object());
    }

    #[test]
    fn subscribe_show_open_file_mode() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("tok".into(), tx);
        let state = test_state(manager);

        let data = json!({"id": "abc123", "data": {"properties": ["openFile"]}});
        handle_subscribe_show_open(&state, conn_id, data);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["name"], "show-open-request");
                assert_eq!(parsed["data"]["id"], "abc123");
                assert_eq!(parsed["data"]["isFileMode"], true);
                assert_eq!(parsed["data"]["properties"], json!(["openFile"]));
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn subscribe_show_open_directory_mode() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("tok".into(), tx);
        let state = test_state(manager);

        let data = json!({"id": "dir1", "data": {"properties": ["openDirectory"]}});
        handle_subscribe_show_open(&state, conn_id, data);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["data"]["id"], "dir1");
                assert_eq!(parsed["data"]["isFileMode"], false);
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn subscribe_show_open_mixed_mode() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("tok".into(), tx);
        let state = test_state(manager);

        let data = json!({"id": "mixed", "data": {"properties": ["openFile", "openDirectory"]}});
        handle_subscribe_show_open(&state, conn_id, data);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["data"]["id"], "mixed");
                assert_eq!(parsed["data"]["isFileMode"], false);
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn subscribe_show_open_empty_properties() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("tok".into(), tx);
        let state = test_state(manager);

        let data = json!({"id": "empty", "data": {"properties": []}});
        handle_subscribe_show_open(&state, conn_id, data);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["data"]["id"], "empty");
                assert_eq!(parsed["data"]["isFileMode"], false);
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn subscribe_show_open_missing_properties() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("tok".into(), tx);
        let state = test_state(manager);

        handle_subscribe_show_open(&state, conn_id, json!({"id": "noprops", "data": {}}));

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["data"]["id"], "noprops");
                assert_eq!(parsed["data"]["isFileMode"], false);
                assert_eq!(parsed["data"]["properties"], json!([]));
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn subscribe_show_open_missing_id_falls_back_to_empty_string() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("tok".into(), tx);
        let state = test_state(manager);

        handle_subscribe_show_open(&state, conn_id, json!({}));

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["data"]["id"], "");
                assert_eq!(parsed["data"]["isFileMode"], false);
                assert_eq!(parsed["data"]["properties"], json!([]));
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn text_message_pong_updates_last_ping() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, _rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("tok".into(), tx);
        let state = test_state(manager);

        std::thread::sleep(std::time::Duration::from_millis(5));

        handle_text_message(conn_id, r#"{"name":"pong","data":{}}"#, &state);
        // No panic = success (update_last_ping was called)
    }

    #[test]
    fn text_message_invalid_json_sends_error() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("tok".into(), tx);
        let state = test_state(manager);

        handle_text_message(conn_id, "not json", &state);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                assert_invalid_message_error(&text);
            }
            _ => panic!("expected error text"),
        }
    }

    #[test]
    fn text_message_missing_fields_sends_error() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("tok".into(), tx);
        let state = test_state(manager);

        handle_text_message(conn_id, r#"{"foo":"bar"}"#, &state);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                assert_invalid_message_error(&text);
            }
            _ => panic!("expected error text"),
        }
    }

    #[test]
    fn text_message_empty_name_sends_error() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("tok".into(), tx);
        let state = test_state(manager);

        handle_text_message(conn_id, r#"{"name":"","data":{}}"#, &state);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                assert_invalid_message_error(&text);
            }
            _ => panic!("expected error text"),
        }
    }

    #[test]
    fn text_message_routes_unknown_to_router() {
        use std::sync::atomic::{AtomicBool, Ordering};

        struct TestRouter {
            called: AtomicBool,
        }
        impl MessageRouter for TestRouter {
            fn route(&self, _conn_id: ConnectionId, _name: &str, _data: Value) -> bool {
                self.called.store(true, Ordering::Relaxed);
                true
            }
        }

        let manager = Arc::new(WebSocketManager::new());
        let (tx, _rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("tok".into(), tx);

        let router = Arc::new(TestRouter {
            called: AtomicBool::new(false),
        });
        let state = WsHandlerState {
            manager,
            router: router.clone(),
            token_validator: Arc::new(|_| true),
            token_extractor: Arc::new(|_| None),
        };

        handle_text_message(
            conn_id,
            r#"{"name":"conversation.send-message","data":{"text":"hi"}}"#,
            &state,
        );

        assert!(router.called.load(Ordering::Relaxed));
    }

    #[test]
    fn text_message_unhandled_by_router_sends_unsupported_error() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("tok".into(), tx);
        let state = test_state(manager);

        handle_text_message(
            conn_id,
            r#"{"name":"conversation.send-message","data":{"text":"hi"}}"#,
            &state,
        );

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                assert_unsupported_message_error(&text);
            }
            _ => panic!("expected unsupported message error"),
        }
    }

    #[test]
    fn error_response_to_disconnected_client_is_noop() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("tok".into(), tx);
        drop(rx); // close channel

        let state = test_state(manager.clone());

        // Should not panic — client will be removed
        send_error_response(&state, conn_id);
        assert_eq!(manager.client_count(), 0);
    }
}
