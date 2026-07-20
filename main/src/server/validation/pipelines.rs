use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use previa_runner::{Pipeline, RuntimeEnvGroup, RuntimeSpec};
use regex::Regex;
use serde_json::Value;

pub const KNOWN_TEMPLATE_HELPERS: &[&str] = &[
    "uuid", "email", "name", "username", "number", "date", "boolean", "cpf",
];

pub fn validate_pipeline_templates(
    pipeline: &Pipeline,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
) -> Vec<String> {
    let specs_index = build_specs_index(specs);
    let env_groups_index = build_env_groups_index(env_groups);
    let mut known_steps = HashSet::new();
    let mut errors = Vec::new();

    for step in &pipeline.steps {
        validate_string_templates(
            &step.url,
            &format!("step '{}' field 'url'", step.id),
            &known_steps,
            &specs_index,
            &env_groups_index,
            selected_env_group_slug,
            &mut errors,
        );

        for (header_name, header_value) in &step.headers {
            validate_string_templates(
                header_value,
                &format!("step '{}' header '{}'", step.id, header_name),
                &known_steps,
                &specs_index,
                &env_groups_index,
                selected_env_group_slug,
                &mut errors,
            );
        }

        if let Some(body) = step.body.as_ref() {
            validate_value_templates(
                body,
                &format!("step '{}' body", step.id),
                &known_steps,
                &specs_index,
                &env_groups_index,
                selected_env_group_slug,
                &mut errors,
            );
        }

        for (index, assertion) in step.asserts.iter().enumerate() {
            if let Some(expected) = assertion.expected.as_ref() {
                validate_string_templates(
                    expected,
                    &format!("step '{}' assertion {} expected value", step.id, index + 1),
                    &known_steps,
                    &specs_index,
                    &env_groups_index,
                    selected_env_group_slug,
                    &mut errors,
                );
            }
        }

        known_steps.insert(step.id.clone());
    }

    errors
}

fn validate_value_templates(
    value: &Value,
    path: &str,
    known_steps: &HashSet<String>,
    specs_index: &HashMap<String, HashSet<String>>,
    env_groups_index: &HashMap<String, HashSet<String>>,
    selected_env_group_slug: Option<&str>,
    errors: &mut Vec<String>,
) {
    match value {
        Value::String(value) => {
            validate_string_templates(
                value,
                path,
                known_steps,
                specs_index,
                env_groups_index,
                selected_env_group_slug,
                errors,
            );
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                validate_value_templates(
                    item,
                    &format!("{path}[{index}]"),
                    known_steps,
                    specs_index,
                    env_groups_index,
                    selected_env_group_slug,
                    errors,
                );
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                validate_value_templates(
                    item,
                    &format!("{path}.{key}"),
                    known_steps,
                    specs_index,
                    env_groups_index,
                    selected_env_group_slug,
                    errors,
                );
            }
        }
        _ => {}
    }
}

fn validate_string_templates(
    value: &str,
    path: &str,
    known_steps: &HashSet<String>,
    specs_index: &HashMap<String, HashSet<String>>,
    env_groups_index: &HashMap<String, HashSet<String>>,
    selected_env_group_slug: Option<&str>,
    errors: &mut Vec<String>,
) {
    for expression in template_regex().captures_iter(value) {
        let raw_expression = expression
            .get(1)
            .map(|capture| capture.as_str().trim())
            .unwrap_or_default();
        if raw_expression.is_empty() {
            errors.push(format!("{path} uses an empty template expression"));
            continue;
        }

        let Some(normalized) = normalize_expression(raw_expression) else {
            errors.push(format!(
                "{path} uses an invalid template expression '{{{{{raw_expression}}}}}'"
            ));
            continue;
        };

        if let Some(message) = validate_expression(
            &normalized,
            known_steps,
            specs_index,
            env_groups_index,
            selected_env_group_slug,
        ) {
            errors.push(format!("{path}: {message}"));
        }
    }
}

fn validate_expression(
    expression: &str,
    known_steps: &HashSet<String>,
    specs_index: &HashMap<String, HashSet<String>>,
    env_groups_index: &HashMap<String, HashSet<String>>,
    selected_env_group_slug: Option<&str>,
) -> Option<String> {
    if let Some(helper_expression) = expression.strip_prefix("helpers.") {
        let helper_name = helper_expression
            .split_whitespace()
            .next()
            .unwrap_or_default();
        return if is_known_helper(helper_name) {
            None
        } else {
            Some(format!(
                "template variable '{{{{{expression}}}}}' uses unknown helper '{helper_name}'"
            ))
        };
    }

    if let Some(step_expression) = expression.strip_prefix("steps.") {
        let step_id = step_expression.split('.').next().unwrap_or_default().trim();
        if step_id.is_empty() {
            return Some(format!(
                "template variable '{{{{{expression}}}}}' does not define a step id"
            ));
        }
        return if known_steps.contains(step_id) {
            None
        } else {
            Some(format!(
                "template variable '{{{{{expression}}}}}' references step '{step_id}' that is not available yet"
            ))
        };
    }

    if let Some(spec_expression) = expression.strip_prefix("specs.") {
        let mut parts = spec_expression.split('.');
        let slug = parts.next().unwrap_or_default().trim();
        let group = parts.next().unwrap_or_default().trim();
        let url_name = parts.next().unwrap_or_default().trim();

        if slug.is_empty() || group != "url" || url_name.is_empty() {
            return Some(format!(
                "template variable '{{{{{expression}}}}}' must use the format '{{{{specs.<slug>.url.<name>}}}}'"
            ));
        }

        let Some(urls) = specs_index.get(slug) else {
            return Some(format!(
                "template variable '{{{{{expression}}}}}' references unknown spec slug '{slug}'"
            ));
        };

        return if urls.contains(url_name) {
            None
        } else {
            Some(format!(
                "template variable '{{{{{expression}}}}}' references unknown spec url '{url_name}' for slug '{slug}'"
            ))
        };
    }

    if let Some(env_expression) = expression.strip_prefix("envs.") {
        let mut parts = env_expression.split('.');
        let group = parts.next().unwrap_or_default().trim();
        let name = parts.next().unwrap_or_default().trim();

        if group.is_empty() || name.is_empty() || parts.next().is_some() {
            return Some(format!(
                "template variable '{{{{{expression}}}}}' must use the format '{{{{envs.<group>.<name>}}}}' or '{{{{envs.current.<name>}}}}'"
            ));
        }

        if env_groups_index.is_empty() && selected_env_group_slug.is_none() {
            return None;
        }

        let resolved_group = if group == "current" {
            let Some(selected) = selected_env_group_slug
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return Some(format!(
                    "template variable '{{{{{expression}}}}}' requires a selected env group"
                ));
            };
            selected
        } else {
            group
        };

        let Some(entries) = env_groups_index.get(resolved_group) else {
            return Some(format!(
                "template variable '{{{{{expression}}}}}' references unknown env group '{resolved_group}'"
            ));
        };

        return if entries.contains(name) {
            None
        } else {
            Some(format!(
                "template variable '{{{{{expression}}}}}' references unknown env '{name}' for group '{resolved_group}'"
            ))
        };
    }

    Some(format!(
        "template variable '{{{{{expression}}}}}' does not exist"
    ))
}

fn build_env_groups_index(
    env_groups: Option<&[RuntimeEnvGroup]>,
) -> HashMap<String, HashSet<String>> {
    let mut index = HashMap::new();

    for group in env_groups.unwrap_or(&[]) {
        let slug = group.slug.trim();
        if slug.is_empty() {
            continue;
        }

        let mut urls = HashSet::new();
        for name in group.urls.keys() {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                urls.insert(trimmed.to_owned());
            }
        }

        index.insert(slug.to_owned(), urls);
    }

    index
}

fn build_specs_index(specs: Option<&[RuntimeSpec]>) -> HashMap<String, HashSet<String>> {
    let mut index = HashMap::new();

    for spec in specs.unwrap_or(&[]) {
        if spec.slug.trim().is_empty() {
            continue;
        }

        let mut urls = HashSet::new();
        for name in spec.servers.keys() {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                urls.insert(trimmed.to_owned());
            }
        }

        index.insert(spec.slug.trim().to_owned(), urls);
    }

    index
}

fn is_known_helper(name: &str) -> bool {
    KNOWN_TEMPLATE_HELPERS.contains(&name)
}

fn normalize_expression(expression: &str) -> Option<String> {
    if let Some(rest) = expression.strip_prefix("url.") {
        let parts: Vec<&str> = rest.split('.').collect();
        if parts.len() >= 2 {
            return Some(format!("specs.{}.url.{}", parts[0], parts[1]));
        }
        return None;
    }

    Some(expression.to_owned())
}

fn template_regex() -> &'static Regex {
    static TEMPLATE_REGEX: OnceLock<Regex> = OnceLock::new();
    TEMPLATE_REGEX.get_or_init(|| Regex::new(r"\{\{([^}]+)\}\}").expect("valid regex"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use previa_runner::{Pipeline, PipelineStep, RuntimeEnvGroup, RuntimeSpec};
    use serde_json::json;

    use super::validate_pipeline_templates;

    fn sample_step(id: &str, url: &str) -> PipelineStep {
        PipelineStep {
            id: id.to_owned(),
            name: id.to_owned(),
            description: None,
            method: "GET".to_owned(),
            url: url.to_owned(),
            headers: HashMap::new(),
            body: None,
            operation_id: None,
            delay: None,
            retry: None,
            extracts: Vec::new(),
            asserts: Vec::new(),
        }
    }

    #[test]
    fn rejects_unknown_root_variable() {
        let pipeline = Pipeline {
            id: None,
            name: "test".to_owned(),
            description: None,
            steps: vec![sample_step("step-1", "https://example.com/{{run.id}}")],
        };

        let errors = validate_pipeline_templates(&pipeline, None, None, None);
        assert!(errors.iter().any(|item| item.contains("{{run.id}}")));
    }

    #[test]
    fn rejects_future_step_reference() {
        let pipeline = Pipeline {
            id: None,
            name: "test".to_owned(),
            description: None,
            steps: vec![
                sample_step("step-1", "https://example.com/{{steps.step-2.id}}"),
                sample_step("step-2", "https://example.com"),
            ],
        };

        let errors = validate_pipeline_templates(&pipeline, None, None, None);
        assert!(errors.iter().any(|item| item.contains("step 'step-2'")));
    }

    #[test]
    fn accepts_known_step_and_spec_references() {
        let mut second = sample_step("step-2", "https://example.com/{{steps.step-1.id}}");
        second.body = Some(json!({
            "baseUrl": "{{specs.payments.url.hml}}",
            "requestId": "{{helpers.uuid}}"
        }));

        let pipeline = Pipeline {
            id: None,
            name: "test".to_owned(),
            description: None,
            steps: vec![sample_step("step-1", "https://example.com"), second],
        };
        let specs = vec![RuntimeSpec {
            slug: "payments".to_owned(),
            servers: HashMap::from([("hml".to_owned(), "https://hml.example.com".to_owned())]),
        }];

        let errors = validate_pipeline_templates(&pipeline, Some(&specs), None, None);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn accepts_current_env_reference_when_selected_group_has_entry() {
        let pipeline = Pipeline {
            id: None,
            name: "test".to_owned(),
            description: None,
            steps: vec![sample_step("step-1", "{{envs.current.api}}/health")],
        };
        let env_groups = vec![RuntimeEnvGroup {
            slug: "hml".to_owned(),
            urls: HashMap::from([("api".to_owned(), "https://api-hml.example.com".to_owned())]),
        }];

        let errors = validate_pipeline_templates(&pipeline, None, Some(&env_groups), Some("hml"));
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn rejects_current_env_reference_without_selected_group() {
        let pipeline = Pipeline {
            id: None,
            name: "test".to_owned(),
            description: None,
            steps: vec![sample_step("step-1", "{{envs.current.api}}/health")],
        };
        let env_groups = vec![RuntimeEnvGroup {
            slug: "hml".to_owned(),
            urls: HashMap::from([("api".to_owned(), "https://api-hml.example.com".to_owned())]),
        }];

        let errors = validate_pipeline_templates(&pipeline, None, Some(&env_groups), None);
        assert!(
            errors
                .iter()
                .any(|item| item.contains("requires a selected env group")),
            "unexpected errors: {errors:?}"
        );
    }

    #[test]
    fn rejects_unknown_env_entry() {
        let pipeline = Pipeline {
            id: None,
            name: "test".to_owned(),
            description: None,
            steps: vec![sample_step("step-1", "{{envs.hml.auth}}/health")],
        };
        let env_groups = vec![RuntimeEnvGroup {
            slug: "hml".to_owned(),
            urls: HashMap::from([("api".to_owned(), "https://api-hml.example.com".to_owned())]),
        }];

        let errors = validate_pipeline_templates(&pipeline, None, Some(&env_groups), Some("hml"));
        assert!(
            errors
                .iter()
                .any(|item| item.contains("unknown env 'auth'")),
            "unexpected errors: {errors:?}"
        );
    }
}
