use serde_json::Value;
use std::collections::HashMap;

use crate::core::types::{
    AssertionResult, PipelineStep, RuntimeEnvGroup, RuntimeSpec, StepExecutionResult,
};
use crate::template::resolve::{resolve_template_variables, value_to_string};

pub(crate) fn has_status_assertion(step: &PipelineStep) -> bool {
    step.asserts
        .iter()
        .any(|assertion| assertion.field == "status")
}

pub(crate) fn resolve_assert_field(field: &str, result: &StepExecutionResult) -> Option<String> {
    let response = result.response.as_ref()?;

    if field == "status" {
        return Some(response.status.to_string());
    }

    if let Some(path) = field.strip_prefix("body.") {
        return resolve_json_path(&response.body, path).and_then(value_to_string);
    }

    if let Some(header_name) = field.strip_prefix("header.") {
        for (k, v) in &response.headers {
            if k.eq_ignore_ascii_case(header_name) {
                return Some(v.clone());
            }
        }
    }

    None
}

fn resolve_json_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => {
                let index = segment.parse::<usize>().ok()?;
                items.get(index)?
            }
            _ => return None,
        };
    }
    Some(current)
}

pub(crate) fn evaluate_assertions(
    step: &PipelineStep,
    result: &StepExecutionResult,
    context: &HashMap<String, StepExecutionResult>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
) -> Vec<AssertionResult> {
    step.asserts
        .iter()
        .map(|assertion| {
            let actual = resolve_assert_field(&assertion.field, result);
            let expected = assertion.expected.as_ref().map(|exp| {
                resolve_template_variables(
                    &Value::String(exp.clone()),
                    context,
                    specs,
                    env_groups,
                    selected_env_group_slug,
                )
                .as_str()
                .unwrap_or(exp)
                .to_owned()
            });

            let passed = match assertion.operator.as_str() {
                "equals" => actual == expected,
                "not_equals" => actual != expected,
                "contains" => match (actual.as_ref(), expected.as_ref()) {
                    (Some(a), Some(e)) => a.contains(e),
                    _ => false,
                },
                "exists" => actual.is_some(),
                "not_exists" => actual.is_none(),
                "gt" => match (actual.as_ref(), expected.as_ref()) {
                    (Some(a), Some(e)) => {
                        let left = a.parse::<f64>().ok();
                        let right = e.parse::<f64>().ok();
                        matches!((left, right), (Some(l), Some(r)) if l > r)
                    }
                    _ => false,
                },
                "lt" => match (actual.as_ref(), expected.as_ref()) {
                    (Some(a), Some(e)) => {
                        let left = a.parse::<f64>().ok();
                        let right = e.parse::<f64>().ok();
                        matches!((left, right), (Some(l), Some(r)) if l < r)
                    }
                    _ => false,
                },
                _ => false,
            };

            AssertionResult {
                assertion: assertion.clone(),
                passed,
                actual,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::resolve_assert_field;
    use crate::core::types::{StepExecutionResult, StepResponse};
    use serde_json::{Value, json};
    use std::collections::HashMap;

    fn step_result(body: Value) -> StepExecutionResult {
        StepExecutionResult {
            step_id: "step".to_owned(),
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
            extracts: HashMap::new(),
            assert_results: None,
        }
    }

    #[test]
    fn resolves_assert_fields_inside_arrays() {
        let result = step_result(json!({
            "pods": [
                {
                    "phase": "Running",
                    "podName": "pod-1"
                }
            ],
            "containers": [
                {
                    "name": "app"
                }
            ]
        }));

        assert_eq!(
            resolve_assert_field("body.pods.0.phase", &result),
            Some("Running".to_owned())
        );
        assert_eq!(
            resolve_assert_field("body.pods.0.podName", &result),
            Some("pod-1".to_owned())
        );
        assert_eq!(
            resolve_assert_field("body.containers.0.name", &result),
            Some("app".to_owned())
        );
    }

    #[test]
    fn returns_none_for_invalid_array_indexes() {
        let result = step_result(json!({
            "pods": [
                {
                    "phase": "Running"
                }
            ]
        }));

        assert_eq!(resolve_assert_field("body.pods.one.phase", &result), None);
        assert_eq!(resolve_assert_field("body.pods.2.phase", &result), None);
    }
}
