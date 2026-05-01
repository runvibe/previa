use std::collections::HashSet;

use crate::server::models::{EnvGroupEntry, ProjectEnvGroupUpsertRequest};

pub fn normalize_env_group_payload(
    mut payload: ProjectEnvGroupUpsertRequest,
) -> Result<ProjectEnvGroupUpsertRequest, &'static str> {
    payload.slug = normalize_env_slug(&payload.slug)?;
    payload.name = payload.name.trim().to_owned();
    if payload.name.is_empty() {
        return Err("env group name is required");
    }
    payload.entries = normalize_env_entries(payload.entries)?;
    Ok(payload)
}

pub fn normalize_env_slug(raw: &str) -> Result<String, &'static str> {
    let value = raw.trim();
    if value.is_empty() {
        return Err("env group slug is required");
    }
    if value == "current" {
        return Err("env group slug 'current' is reserved");
    }
    if value.starts_with('-') || value.ends_with('-') || value.contains("--") {
        return Err("env group slug cannot start/end with '-' or contain repeated separators");
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        return Err("env group slug must use lowercase letters, numbers, or '-'");
    }
    Ok(value.to_owned())
}

pub fn normalize_env_entries(
    entries: Vec<EnvGroupEntry>,
) -> Result<Vec<EnvGroupEntry>, &'static str> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::with_capacity(entries.len());
    for entry in entries {
        let name = entry.name.trim().to_ascii_lowercase();
        if name.is_empty() {
            return Err("env entries[].name is required");
        }
        if !is_valid_env_entry_name(&name) {
            return Err("env entries[].name must use lowercase letters, numbers, '-' or '_'");
        }
        if !seen.insert(name.clone()) {
            return Err("env entries[].name must be unique");
        }
        let url = entry.url.trim().to_owned();
        if url.is_empty() {
            return Err("env entries[].url is required");
        }
        normalized.push(EnvGroupEntry {
            name,
            url,
            description: entry
                .description
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
        });
    }
    Ok(normalized)
}

fn is_valid_env_entry_name(name: &str) -> bool {
    if name.starts_with('-')
        || name.ends_with('-')
        || name.starts_with('_')
        || name.ends_with('_')
        || name.contains("--")
        || name.contains("__")
        || name.contains("-_")
        || name.contains("_-")
    {
        return false;
    }
    name.chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
}

#[cfg(test)]
mod tests {
    use crate::server::models::{EnvGroupEntry, ProjectEnvGroupUpsertRequest};
    use crate::server::validation::env_groups::{
        normalize_env_entries, normalize_env_group_payload, normalize_env_slug,
    };

    #[test]
    fn rejects_reserved_current_slug() {
        let err = normalize_env_slug("current").expect_err("current should be reserved");
        assert!(err.contains("reserved"));
    }

    #[test]
    fn normalizes_payload_entries() {
        let payload = normalize_env_group_payload(ProjectEnvGroupUpsertRequest {
            slug: " hml ".to_owned(),
            name: " Homolog ".to_owned(),
            entries: vec![EnvGroupEntry {
                name: " API ".to_owned(),
                url: " https://api.example.com ".to_owned(),
                description: Some(" Main API ".to_owned()),
            }],
        })
        .expect("payload should normalize");

        assert_eq!(payload.slug, "hml");
        assert_eq!(payload.name, "Homolog");
        assert_eq!(payload.entries[0].name, "api");
        assert_eq!(payload.entries[0].url, "https://api.example.com");
        assert_eq!(payload.entries[0].description.as_deref(), Some("Main API"));
    }

    #[test]
    fn rejects_duplicate_entry_names() {
        let err = normalize_env_entries(vec![
            EnvGroupEntry {
                name: "api".to_owned(),
                url: "https://a.example.com".to_owned(),
                description: None,
            },
            EnvGroupEntry {
                name: "API".to_owned(),
                url: "https://b.example.com".to_owned(),
                description: None,
            },
        ])
        .expect_err("duplicate names should fail");

        assert!(err.contains("unique"));
    }
}
