use std::collections::{HashMap, HashSet};

use regex::Regex;
use serde_json::Value;

use crate::core::types::{PipelineStep, StepExecutionResult};

pub fn validate_step_extractions(step: &PipelineStep) -> Vec<String> {
    let mut names = HashSet::new();
    let mut errors = Vec::new();

    for extraction in &step.extracts {
        if !is_valid_name(&extraction.name) {
            errors.push(format!(
                "extraction '{}' has an invalid name; use lowercase letters, digits, '-' or '_'",
                extraction.name
            ));
        }
        if !names.insert(extraction.name.as_str()) {
            errors.push(format!(
                "extraction '{}' is a duplicate within step '{}'",
                extraction.name, step.id
            ));
        }
        if extraction.field != "body"
            && (!extraction.field.starts_with("body.") || extraction.field == "body.")
        {
            errors.push(format!(
                "extraction '{}' field must be 'body' or start with 'body.'",
                extraction.name
            ));
        }

        match Regex::new(&extraction.regex) {
            Ok(regex) if extraction.group >= regex.captures_len() => errors.push(format!(
                "extraction '{}' group {} does not exist in regex",
                extraction.name, extraction.group
            )),
            Ok(_) => {}
            Err(_) => errors.push(format!(
                "extraction '{}' has an invalid regex",
                extraction.name
            )),
        }
    }

    errors
}

pub fn evaluate_step_extractions(
    step: &PipelineStep,
    result: &StepExecutionResult,
) -> Result<HashMap<String, String>, String> {
    let validation_errors = validate_step_extractions(step);
    if !validation_errors.is_empty() {
        return Err(validation_errors.join("; "));
    }

    let mut values = HashMap::new();
    for extraction in &step.extracts {
        let captured = resolve_source(&extraction.field, result).and_then(|source| {
            let regex = Regex::new(&extraction.regex).ok()?;
            regex
                .captures(&source)
                .and_then(|captures| captures.get(extraction.group))
                .map(|capture| capture.as_str().to_owned())
        });

        match captured {
            Some(value) => {
                values.insert(extraction.name.clone(), value);
            }
            None if extraction.required => {
                return Err(format!(
                    "required extraction '{}' did not produce a value",
                    extraction.name
                ));
            }
            None => {}
        }
    }

    Ok(values)
}

fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
}

fn resolve_source(field: &str, result: &StepExecutionResult) -> Option<String> {
    let response = result.response.as_ref()?;
    let value = if field == "body" {
        &response.body
    } else {
        resolve_json_path(&response.body, field.strip_prefix("body.")?)?
    };
    value_to_string(value)
}

fn resolve_json_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use crate::{
        PipelineStep, StepExecutionResult, StepExtraction, StepResponse, evaluate_step_extractions,
        validate_step_extractions,
    };

    fn step(extracts: Vec<StepExtraction>) -> PipelineStep {
        PipelineStep {
            id: "email".to_owned(),
            name: "Read e-mail".to_owned(),
            description: None,
            method: "GET".to_owned(),
            url: "https://example.test/message".to_owned(),
            headers: HashMap::new(),
            body: None,
            operation_id: None,
            delay: None,
            retry: None,
            asserts: Vec::new(),
            extracts,
        }
    }

    fn result(body: serde_json::Value) -> StepExecutionResult {
        StepExecutionResult {
            step_id: "email".to_owned(),
            status: "success".to_owned(),
            request: None,
            response: Some(StepResponse {
                status: 200,
                status_text: "OK".to_owned(),
                headers: HashMap::new(),
                body,
            }),
            error: None,
            duration: Some(1),
            attempts: None,
            attempt: Some(1),
            max_attempts: Some(1),
            assert_results: None,
            extracts: HashMap::new(),
        }
    }

    fn extraction(name: &str, field: &str, regex: &str) -> StepExtraction {
        StepExtraction {
            name: name.to_owned(),
            field: field.to_owned(),
            regex: regex.to_owned(),
            group: 1,
            required: true,
        }
    }

    #[test]
    fn extracts_capture_from_nested_json_string() {
        let step = step(vec![extraction(
            "code",
            "body.HTML",
            r"<strong>[[:space:]]*([0-9]{6})[[:space:]]*</strong>",
        )]);
        let result = result(json!({"HTML": "<p><strong>123456</strong></p>"}));

        assert_eq!(
            evaluate_step_extractions(&step, &result)
                .expect("capture should succeed")
                .get("code"),
            Some(&"123456".to_owned())
        );
    }

    #[test]
    fn group_zero_extracts_the_entire_match_from_string_body() {
        let mut definition = extraction("code", "body", r"[0-9]{6}");
        definition.group = 0;
        let step = step(vec![definition]);
        let result = result(json!("Login code: 123456"));

        assert_eq!(
            evaluate_step_extractions(&step, &result)
                .expect("capture should succeed")
                .get("code"),
            Some(&"123456".to_owned())
        );
    }

    #[test]
    fn missing_optional_capture_is_omitted() {
        let mut definition = extraction("code", "body.HTML", r"([0-9]{6})");
        definition.required = false;
        let step = step(vec![definition]);
        let result = result(json!({"HTML": "no code"}));

        assert!(
            evaluate_step_extractions(&step, &result)
                .expect("optional capture should not fail")
                .is_empty()
        );
    }

    #[test]
    fn missing_required_capture_fails_without_response_content() {
        let step = step(vec![extraction("code", "body.HTML", r"([0-9]{6})")]);
        let result = result(json!({"HTML": "sensitive message without code"}));

        let error =
            evaluate_step_extractions(&step, &result).expect_err("required capture should fail");

        assert!(error.contains("code"));
        assert!(!error.contains("sensitive message"));
    }

    #[test]
    fn validates_invalid_regex_duplicate_and_invalid_names() {
        let step = step(vec![
            extraction("bad name", "body.HTML", "("),
            extraction("bad name", "body.HTML", r"([0-9]{6})"),
        ]);

        let errors = validate_step_extractions(&step);

        assert!(errors.iter().any(|error| error.contains("invalid name")));
        assert!(errors.iter().any(|error| error.contains("invalid regex")));
        assert!(errors.iter().any(|error| error.contains("duplicate")));
    }

    #[test]
    fn validates_source_path_and_capture_group() {
        let invalid_path = extraction("code", "header.subject", r"([0-9]{6})");
        let mut invalid_group = extraction("token", "body.Text", r"([a-z]+)");
        invalid_group.group = 2;
        let step = step(vec![invalid_path, invalid_group]);

        let errors = validate_step_extractions(&step);

        assert!(errors.iter().any(|error| error.contains("field")));
        assert!(errors.iter().any(|error| error.contains("group")));
    }
}
