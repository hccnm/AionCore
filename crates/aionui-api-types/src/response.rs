#![allow(clippy::disallowed_types)]

use serde::{Deserialize, Serialize};

/// Standard API success response envelope.
///
/// All REST endpoints should return this envelope at the HTTP boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub code: i32,
    pub message: String,
    pub data: Option<T>,
    pub trace_id: Option<String>,
}

impl<T> ApiResponse<T> {
    /// Create a success response with data.
    pub fn ok(data: T) -> Self {
        Self {
            code: 0,
            message: "ok".to_owned(),
            data: Some(data),
            trace_id: None,
        }
    }

    /// Create a success response with data and a message.
    pub fn with_message(data: T, message: impl Into<String>) -> Self {
        Self {
            code: 0,
            message: message.into(),
            data: Some(data),
            trace_id: None,
        }
    }
}

impl ApiResponse<()> {
    /// Create a success response with only a message (no data payload).
    pub fn message(msg: impl Into<String>) -> Self {
        Self {
            code: 0,
            message: msg.into(),
            data: None,
            trace_id: None,
        }
    }

    /// Create a minimal success response (no data, no message).
    pub fn success() -> Self {
        Self {
            code: 0,
            message: "ok".to_owned(),
            data: None,
            trace_id: None,
        }
    }
}

/// Standard API error response.
///
/// Matches the unified JSON response envelope:
/// `{ "code": 400, "message": "...", "data": ..., "trace_id": ... }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: i32,
    pub message: String,
    pub data: Option<serde_json::Value>,
    pub trace_id: Option<String>,
}

impl ErrorResponse {
    pub fn new(message: impl Into<String>, code: i32) -> Self {
        Self::new_with_details(message, code, None)
    }

    pub fn new_with_details(message: impl Into<String>, code: i32, data: impl Into<Option<serde_json::Value>>) -> Self {
        Self {
            code,
            message: message.into(),
            data: data.into(),
            trace_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_response_ok() {
        let resp = ApiResponse::ok(42);
        assert_eq!(resp.code, 0);
        assert_eq!(resp.message, "ok");
        assert_eq!(resp.data, Some(42));
        assert!(resp.trace_id.is_none());
    }

    #[test]
    fn test_api_response_with_message() {
        let resp = ApiResponse::with_message("data", "Created");
        assert_eq!(resp.code, 0);
        assert_eq!(resp.data, Some("data"));
        assert_eq!(resp.message, "Created");
    }

    #[test]
    fn test_api_response_message_only() {
        let resp = ApiResponse::message("Done");
        assert_eq!(resp.code, 0);
        assert!(resp.data.is_none());
        assert_eq!(resp.message, "Done");
    }

    #[test]
    fn test_api_response_success_minimal() {
        let resp = ApiResponse::success();
        assert_eq!(resp.code, 0);
        assert!(resp.data.is_none());
        assert_eq!(resp.message, "ok");
    }

    #[test]
    fn test_api_response_serialization_with_data() {
        let resp = ApiResponse::ok("hello");
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["code"], 0);
        assert_eq!(json["message"], "ok");
        assert_eq!(json["data"], "hello");
        assert!(json.get("trace_id").is_some());
    }

    #[test]
    fn test_api_response_serialization_message_only() {
        let resp = ApiResponse::message("Logged out successfully");
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["code"], 0);
        assert_eq!(json["data"], serde_json::Value::Null);
        assert_eq!(json["message"], "Logged out successfully");
    }

    #[test]
    fn test_api_response_serialization_minimal() {
        let resp = ApiResponse::success();
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["code"], 0);
        assert_eq!(json["message"], "ok");
        assert_eq!(json["data"], serde_json::Value::Null);
    }

    #[test]
    fn test_error_response_new() {
        let resp = ErrorResponse::new("Not found", 404);
        assert_eq!(resp.code, 404);
        assert_eq!(resp.message, "Not found");
        assert!(resp.data.is_none());
    }

    #[test]
    fn test_error_response_serialization() {
        let resp = ErrorResponse::new("Bad request: missing field", 400);
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["code"], 400);
        assert_eq!(json["message"], "Bad request: missing field");
        assert_eq!(json["data"], serde_json::Value::Null);
        assert!(json.get("trace_id").is_some());
    }

    #[test]
    fn test_error_response_new_with_details() {
        let resp = ErrorResponse::new_with_details(
            "Bad request: invalid workspace",
            400,
            serde_json::json!({ "workspace_path": "/tmp/Archive " }),
        );
        assert_eq!(
            resp.data,
            Some(serde_json::json!({ "workspace_path": "/tmp/Archive " }))
        );
    }

    #[test]
    fn test_api_response_deserialization() {
        let json = r#"{"code":0,"message":"ok","data":"test","trace_id":null}"#;
        let resp: ApiResponse<String> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        assert_eq!(resp.data.as_deref(), Some("test"));
        assert_eq!(resp.message, "ok");
    }

    #[test]
    fn test_error_response_deserialization() {
        let json = r#"{"code":404,"message":"Not found","data":null,"trace_id":null}"#;
        let resp: ErrorResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 404);
        assert_eq!(resp.message, "Not found");
        assert!(resp.data.is_none());
    }

    #[test]
    fn test_error_response_with_details() {
        let resp = ErrorResponse::new_with_details(
            "Command not found: npx",
            502,
            Some(serde_json::json!({ "command": "npx" })),
        );
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["data"]["command"], "npx");
    }
}
