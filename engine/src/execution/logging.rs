use std::collections::HashMap;
use tracing::debug;

use serde_json::Value;

use crate::core::types::{StepRequest, StepResponse};

pub(crate) fn log_step_request(step_id: &str, request: &StepRequest) {
    let headers = redact_headers(&request.headers);
    let body = request.body.as_ref().map(redact_json_secrets);
    debug!(
        "{}",
        serde_json::json!({
            "type": "request",
            "stepId": step_id,
            "method": request.method,
            "path": request.url,
            "headers": headers,
            "body": body
        })
    );
}

pub(crate) fn log_step_response(
    step_id: &str,
    response: Option<&StepResponse>,
    error: Option<&str>,
    extracts: &HashMap<String, String>,
) {
    match response {
        Some(response) => {
            let headers = redact_headers(&response.headers);
            let body = redact_extracted_values(&redact_json_secrets(&response.body), extracts);
            debug!(
                "{}",
                serde_json::json!({
                    "type": "response",
                    "stepId": step_id,
                    "status_code": response.status,
                    "headers": headers,
                    "body": body,
                    "error": error
                })
            );
        }
        None => {
            debug!(
                "{}",
                serde_json::json!({
                    "type": "response",
                    "stepId": step_id,
                    "status_code": serde_json::Value::Null,
                    "headers": serde_json::Value::Null,
                    "body": serde_json::Value::Null,
                    "error": error
                })
            );
        }
    }
}

fn redact_headers(headers: &HashMap<String, String>) -> HashMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            let value = if is_sensitive_key(name) {
                "[REDACTED]".to_owned()
            } else {
                value.clone()
            };
            (name.clone(), value)
        })
        .collect()
}

fn redact_json_secrets(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(redact_json_secrets).collect()),
        Value::Object(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, value)| {
                    let value = if is_sensitive_key(key) {
                        Value::String("[REDACTED]".to_owned())
                    } else {
                        redact_json_secrets(value)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn redact_extracted_values(value: &Value, extracts: &HashMap<String, String>) -> Value {
    match value {
        Value::String(text) => {
            let redacted = extracts
                .values()
                .filter(|extracted| !extracted.is_empty())
                .fold(text.clone(), |current, extracted| {
                    current.replace(extracted, "[REDACTED]")
                });
            Value::String(redacted)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| redact_extracted_values(item, extracts))
                .collect(),
        ),
        Value::Object(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), redact_extracted_values(value, extracts)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn is_sensitive_key(key: &str) -> bool {
    matches!(
        key.chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
            .as_str(),
        "authorization"
            | "cookie"
            | "setcookie"
            | "xapikey"
            | "code"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "password"
            | "secret"
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::{redact_extracted_values, redact_headers, redact_json_secrets};

    #[test]
    fn redacts_sensitive_json_keys_recursively() {
        let sanitized = redact_json_secrets(&json!({
            "code": "123456",
            "accessToken": "access",
            "nested": {"refresh_token": "refresh", "safe": "visible"}
        }));

        assert_eq!(sanitized["code"], "[REDACTED]");
        assert_eq!(sanitized["accessToken"], "[REDACTED]");
        assert_eq!(sanitized["nested"]["refresh_token"], "[REDACTED]");
        assert_eq!(sanitized["nested"]["safe"], "visible");
    }

    #[test]
    fn redacts_sensitive_headers_case_insensitively() {
        let sanitized = redact_headers(&HashMap::from([
            ("Authorization".to_owned(), "Bearer secret".to_owned()),
            ("content-type".to_owned(), "application/json".to_owned()),
            ("Set-Cookie".to_owned(), "session=secret".to_owned()),
        ]));

        assert_eq!(sanitized["Authorization"], "[REDACTED]");
        assert_eq!(sanitized["Set-Cookie"], "[REDACTED]");
        assert_eq!(sanitized["content-type"], "application/json");
    }

    #[test]
    fn redacts_extracted_values_inside_text_responses() {
        let sanitized = redact_extracted_values(
            &json!("<strong>123456</strong>"),
            &HashMap::from([("code".to_owned(), "123456".to_owned())]),
        );

        assert_eq!(sanitized, json!("<strong>[REDACTED]</strong>"));
    }
}
