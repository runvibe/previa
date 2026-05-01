use handlebars::{Handlebars, no_escape};
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::core::types::{RuntimeEnvGroup, RuntimeSpec, StepExecutionResult};
use crate::template::helpers::resolve_helper;

pub(crate) fn resolve_template_variables(
    value: &Value,
    context: &HashMap<String, StepExecutionResult>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
) -> Value {
    let template_context =
        build_template_context(context, specs, env_groups, selected_env_group_slug);
    resolve_template_variables_with_context(value, &template_context)
}

pub(crate) fn resolve_template_variables_with_context(
    value: &Value,
    template_context: &Value,
) -> Value {
    match value {
        Value::String(s) => {
            let replaced = template_regex().replace_all(s, |caps: &regex::Captures<'_>| {
                let expr = caps.get(1).map(|m| m.as_str().trim()).unwrap_or_default();
                resolve_expression(expr, template_context)
                    .unwrap_or_else(|| format!("{{{{{}}}}}", expr))
            });
            Value::String(replaced.into_owned())
        }
        Value::Array(arr) => Value::Array(
            arr.iter()
                .map(|v| resolve_template_variables_with_context(v, template_context))
                .collect(),
        ),
        Value::Object(obj) => {
            let mut out = Map::new();
            for (k, v) in obj {
                out.insert(
                    k.clone(),
                    resolve_template_variables_with_context(v, template_context),
                );
            }
            Value::Object(out)
        }
        _ => value.clone(),
    }
}

pub(crate) fn resolve_expression(expression: &str, template_context: &Value) -> Option<String> {
    if expression.starts_with("helpers.") {
        let helper_expr = expression.trim_start_matches("helpers.");
        return resolve_helper(helper_expr);
    }

    let normalized_expression = normalize_legacy_expression(expression)?;
    let handlebars_expression = normalize_handlebars_expression(&normalized_expression);
    let template = format!("{{{{{}}}}}", handlebars_expression);

    render_handlebars_template(&template, template_context)
}

pub(crate) fn template_regex() -> &'static Regex {
    static TEMPLATE_REGEX: OnceLock<Regex> = OnceLock::new();
    TEMPLATE_REGEX.get_or_init(|| Regex::new(r"\{\{([^}]+)\}\}").expect("valid regex"))
}

pub(crate) fn handlebars_engine() -> &'static Handlebars<'static> {
    static HANDLEBARS: OnceLock<Handlebars<'static>> = OnceLock::new();
    HANDLEBARS.get_or_init(|| {
        let mut handlebars = Handlebars::new();
        handlebars.set_strict_mode(true);
        handlebars.register_escape_fn(no_escape);
        handlebars
    })
}

pub(crate) fn render_handlebars_template(template: &str, context: &Value) -> Option<String> {
    handlebars_engine().render_template(template, context).ok()
}

pub(crate) fn normalize_legacy_expression(expression: &str) -> Option<String> {
    if let Some(rest) = expression.strip_prefix("url.") {
        let parts: Vec<&str> = rest.split('.').collect();
        if parts.len() >= 2 {
            return Some(format!("specs.{}.url.{}", parts[0], parts[1]));
        }
        return None;
    }

    Some(expression.to_owned())
}

pub(crate) fn normalize_handlebars_expression(expression: &str) -> String {
    expression
        .split('.')
        .map(|segment| {
            if segment
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            {
                segment.to_owned()
            } else {
                format!("[{}]", segment)
            }
        })
        .collect::<Vec<String>>()
        .join(".")
}

pub(crate) fn build_template_context(
    steps: &HashMap<String, StepExecutionResult>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
) -> Value {
    let mut root = Map::new();

    let mut steps_map = Map::new();
    for (step_id, result) in steps {
        let step_body = result
            .response
            .as_ref()
            .map(|response| response.body.clone())
            .unwrap_or(Value::Null);
        steps_map.insert(step_id.clone(), step_body);
    }
    root.insert("steps".to_owned(), Value::Object(steps_map));

    let mut specs_map = Map::new();
    if let Some(specs) = specs {
        for spec in specs {
            let slug = spec.slug.trim();
            if slug.is_empty() {
                continue;
            }

            let mut urls_map = Map::new();
            for (name, url) in &spec.servers {
                let name = name.trim();
                let url = url.trim();
                if name.is_empty() || url.is_empty() {
                    continue;
                }
                urls_map.insert(name.to_owned(), Value::String(url.to_owned()));
            }

            let mut spec_entry = Map::new();
            spec_entry.insert("url".to_owned(), Value::Object(urls_map));
            specs_map.insert(slug.to_owned(), Value::Object(spec_entry));
        }
    }
    root.insert("specs".to_owned(), Value::Object(specs_map));

    let mut envs_map = Map::new();
    let selected_slug = selected_env_group_slug
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(env_groups) = env_groups {
        for group in env_groups {
            let slug = group.slug.trim();
            if slug.is_empty() {
                continue;
            }

            let mut urls_map = Map::new();
            for (name, url) in &group.urls {
                let name = name.trim();
                let url = url.trim();
                if name.is_empty() || url.is_empty() {
                    continue;
                }
                urls_map.insert(name.to_owned(), Value::String(url.to_owned()));
            }

            if selected_slug == Some(slug) {
                envs_map.insert("current".to_owned(), Value::Object(urls_map.clone()));
            }
            envs_map.insert(slug.to_owned(), Value::Object(urls_map));
        }
    }
    root.insert("envs".to_owned(), Value::Object(envs_map));

    Value::Object(root)
}

pub(crate) fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => Some(value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::RuntimeEnvGroup;

    #[test]
    fn resolves_explicit_env_group_url_variable() {
        let env_groups = [RuntimeEnvGroup {
            slug: "hml".to_owned(),
            urls: HashMap::from([("api".to_owned(), "https://api-hml.example.com".to_owned())]),
        }];
        let context = build_template_context(&HashMap::new(), None, Some(&env_groups), Some("hml"));
        let rendered = resolve_template_variables_with_context(
            &Value::String("{{envs.hml.api}}/health".to_owned()),
            &context,
        );
        assert_eq!(
            rendered,
            Value::String("https://api-hml.example.com/health".to_owned())
        );
    }

    #[test]
    fn resolves_current_env_group_url_variable() {
        let env_groups = [
            RuntimeEnvGroup {
                slug: "local".to_owned(),
                urls: HashMap::from([("api".to_owned(), "http://localhost:3000".to_owned())]),
            },
            RuntimeEnvGroup {
                slug: "hml".to_owned(),
                urls: HashMap::from([("api".to_owned(), "https://api-hml.example.com".to_owned())]),
            },
        ];
        let context = build_template_context(&HashMap::new(), None, Some(&env_groups), Some("hml"));
        let rendered = resolve_template_variables_with_context(
            &Value::String("{{envs.current.api}}/health".to_owned()),
            &context,
        );
        assert_eq!(
            rendered,
            Value::String("https://api-hml.example.com/health".to_owned())
        );
    }

    #[test]
    fn leaves_current_env_variable_unresolved_without_selection() {
        let env_groups = [RuntimeEnvGroup {
            slug: "hml".to_owned(),
            urls: HashMap::from([("api".to_owned(), "https://api-hml.example.com".to_owned())]),
        }];
        let context = build_template_context(&HashMap::new(), None, Some(&env_groups), None);
        let rendered = resolve_template_variables_with_context(
            &Value::String("{{envs.current.api}}/health".to_owned()),
            &context,
        );
        assert_eq!(
            rendered,
            Value::String("{{envs.current.api}}/health".to_owned())
        );
    }
}
