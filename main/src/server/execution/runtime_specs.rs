use std::collections::HashMap;

use crate::server::db::DbPool;
use previa_runner::{RuntimeEnvGroup, RuntimeSpec};

use crate::server::db::{
    list_project_env_group_records, list_project_spec_records, runtime_env_group_from_record,
};
use crate::server::models::ProjectSpecRecord;

pub async fn load_runtime_specs_for_project(
    db: &DbPool,
    project_id: &str,
) -> Result<Vec<RuntimeSpec>, sqlx::Error> {
    let records = list_project_spec_records(db, project_id).await?;
    let specs = records
        .into_iter()
        .filter_map(runtime_spec_from_record)
        .collect();
    Ok(specs)
}

pub async fn load_runtime_env_groups_for_project(
    db: &DbPool,
    project_id: &str,
) -> Result<Vec<RuntimeEnvGroup>, sqlx::Error> {
    let records = list_project_env_group_records(db, project_id).await?;
    Ok(records
        .iter()
        .filter_map(runtime_env_group_from_record)
        .collect())
}

pub async fn resolve_runtime_env_groups_for_execution(
    db: &DbPool,
    project_id: Option<&str>,
    payload_env_groups: &[RuntimeEnvGroup],
) -> Result<Option<Vec<RuntimeEnvGroup>>, sqlx::Error> {
    let sanitized_payload_env_groups = sanitize_runtime_env_groups(payload_env_groups);
    if !sanitized_payload_env_groups.is_empty() {
        return Ok(Some(sanitized_payload_env_groups));
    }

    if let Some(project_id) = project_id {
        let env_groups = load_runtime_env_groups_for_project(db, project_id).await?;
        if !env_groups.is_empty() {
            return Ok(Some(env_groups));
        }
    }

    Ok(None)
}

pub fn sanitize_runtime_env_groups(env_groups: &[RuntimeEnvGroup]) -> Vec<RuntimeEnvGroup> {
    let mut sanitized = Vec::new();

    for group in env_groups {
        let slug = group.slug.trim();
        if slug.is_empty() || slug == "current" {
            continue;
        }

        let mut urls = HashMap::new();
        for (name, url) in &group.urls {
            let name = name.trim();
            let url = url.trim();
            if name.is_empty() || url.is_empty() {
                continue;
            }
            urls.insert(name.to_owned(), url.to_owned());
        }

        if urls.is_empty() {
            continue;
        }

        sanitized.push(RuntimeEnvGroup {
            slug: slug.to_owned(),
            urls,
        });
    }

    sanitized
}

pub async fn resolve_runtime_specs_for_execution(
    db: &DbPool,
    project_id: Option<&str>,
    payload_specs: &[RuntimeSpec],
) -> Result<Option<Vec<RuntimeSpec>>, sqlx::Error> {
    let sanitized_payload_specs = sanitize_runtime_specs(payload_specs);
    if !sanitized_payload_specs.is_empty() {
        return Ok(Some(sanitized_payload_specs));
    }

    if let Some(project_id) = project_id {
        let specs = load_runtime_specs_for_project(db, project_id).await?;
        if !specs.is_empty() {
            return Ok(Some(specs));
        }
    }

    Ok(None)
}

pub fn sanitize_runtime_specs(specs: &[RuntimeSpec]) -> Vec<RuntimeSpec> {
    let mut sanitized = Vec::new();

    for spec in specs {
        let slug = spec.slug.trim();
        if slug.is_empty() {
            continue;
        }

        let mut servers = HashMap::new();
        for (name, url) in &spec.servers {
            let name = name.trim();
            let url = url.trim();
            if name.is_empty() || url.is_empty() {
                continue;
            }
            servers.insert(name.to_owned(), url.to_owned());
        }

        if servers.is_empty() {
            continue;
        }

        sanitized.push(RuntimeSpec {
            slug: slug.to_owned(),
            servers,
        });
    }

    sanitized
}

pub fn runtime_spec_from_record(record: ProjectSpecRecord) -> Option<RuntimeSpec> {
    let slug = record.slug?.trim().to_owned();
    if slug.is_empty() {
        return None;
    }

    let servers = record.servers;
    if servers.is_empty() {
        return None;
    }

    Some(RuntimeSpec { slug, servers })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use previa_runner::RuntimeSpec;

    use crate::server::execution::runtime_specs::sanitize_runtime_specs;

    #[test]
    fn sanitize_runtime_specs_drops_empty_entries() {
        let specs = vec![
            RuntimeSpec {
                slug: "example-api".to_owned(),
                servers: HashMap::new(),
            },
            RuntimeSpec {
                slug: "".to_owned(),
                servers: HashMap::from([("hml".to_owned(), "https://hml.example.com".to_owned())]),
            },
            RuntimeSpec {
                slug: "users-api".to_owned(),
                servers: HashMap::from([("hml".to_owned(), "https://hml.example.com".to_owned())]),
            },
        ];

        let sanitized = sanitize_runtime_specs(&specs);
        assert_eq!(sanitized.len(), 1);
        assert_eq!(sanitized[0].slug, "users-api");
        assert_eq!(
            sanitized[0].servers.get("hml").map(|value| value.as_str()),
            Some("https://hml.example.com")
        );
    }
}
