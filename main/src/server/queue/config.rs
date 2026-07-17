use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainQueueConfig {
    pub database_url: String,
    pub runner_stale_after: Duration,
    pub job_lease: Duration,
    pub job_max_attempts: u32,
    pub projection_lease: Duration,
    pub projection_poll_interval: Duration,
    pub maintenance_interval: Duration,
    pub retry_backoff_base: Duration,
    pub retry_backoff_max: Duration,
    pub event_retention: Duration,
    pub runner_retention: Duration,
}

impl MainQueueConfig {
    pub fn from_env() -> Result<Self, String> {
        let keys = [
            "DATABASE_URL",
            "ORCHESTRATOR_DATABASE_URL",
            "PREVIA_QUEUE_RUNNER_STALE_AFTER_MS",
            "PREVIA_QUEUE_JOB_LEASE_MS",
            "PREVIA_QUEUE_JOB_MAX_ATTEMPTS",
            "PREVIA_QUEUE_PROJECTION_LEASE_MS",
            "PREVIA_QUEUE_PROJECTION_POLL_INTERVAL_MS",
            "PREVIA_QUEUE_MAINTENANCE_INTERVAL_MS",
            "PREVIA_QUEUE_RETRY_BACKOFF_BASE_MS",
            "PREVIA_QUEUE_RETRY_BACKOFF_MAX_MS",
            "PREVIA_QUEUE_EVENT_RETENTION_HOURS",
            "PREVIA_QUEUE_RUNNER_RETENTION_HOURS",
        ];
        let owned = keys
            .iter()
            .filter_map(|key| std::env::var(key).ok().map(|value| (*key, value)))
            .collect::<Vec<_>>();
        let borrowed = owned
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect::<Vec<_>>();
        Self::from_env_values(&borrowed)
    }

    pub fn from_env_values(values: &[(&str, &str)]) -> Result<Self, String> {
        let values = values.iter().copied().collect::<HashMap<_, _>>();
        let database_url = values
            .get("DATABASE_URL")
            .or_else(|| values.get("ORCHESTRATOR_DATABASE_URL"))
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "DATABASE_URL is required and must use Postgres".to_owned())?;
        require_postgres_url("DATABASE_URL", database_url)?;

        let runner_stale_after = duration_ms(
            &values,
            "PREVIA_QUEUE_RUNNER_STALE_AFTER_MS",
            15_000,
            5_000,
            300_000,
        )?;
        let job_lease = duration_ms(
            &values,
            "PREVIA_QUEUE_JOB_LEASE_MS",
            30_000,
            10_000,
            600_000,
        )?;
        let job_max_attempts = number(&values, "PREVIA_QUEUE_JOB_MAX_ATTEMPTS", 3_u32, 1, 10)?;
        let projection_lease = duration_ms(
            &values,
            "PREVIA_QUEUE_PROJECTION_LEASE_MS",
            30_000,
            10_000,
            300_000,
        )?;
        let projection_poll_interval = duration_ms(
            &values,
            "PREVIA_QUEUE_PROJECTION_POLL_INTERVAL_MS",
            1_000,
            100,
            60_000,
        )?;
        let maintenance_interval = duration_ms(
            &values,
            "PREVIA_QUEUE_MAINTENANCE_INTERVAL_MS",
            1_000,
            100,
            60_000,
        )?;
        let retry_backoff_base = duration_ms(
            &values,
            "PREVIA_QUEUE_RETRY_BACKOFF_BASE_MS",
            1_000,
            100,
            60_000,
        )?;
        let retry_backoff_max = duration_ms(
            &values,
            "PREVIA_QUEUE_RETRY_BACKOFF_MAX_MS",
            30_000,
            1_000,
            600_000,
        )?;
        if retry_backoff_max < retry_backoff_base {
            return Err(
                "PREVIA_QUEUE_RETRY_BACKOFF_MAX_MS must be greater than or equal to PREVIA_QUEUE_RETRY_BACKOFF_BASE_MS"
                    .to_owned(),
            );
        }
        let event_retention =
            duration_hours(&values, "PREVIA_QUEUE_EVENT_RETENTION_HOURS", 24, 1, 720)?;
        let runner_retention = duration_hours(
            &values,
            "PREVIA_QUEUE_RUNNER_RETENTION_HOURS",
            168,
            1,
            8_760,
        )?;

        Ok(Self {
            database_url: database_url.to_owned(),
            runner_stale_after,
            job_lease,
            job_max_attempts,
            projection_lease,
            projection_poll_interval,
            maintenance_interval,
            retry_backoff_base,
            retry_backoff_max,
            event_retention,
            runner_retention,
        })
    }
}

fn require_postgres_url(name: &str, value: &str) -> Result<(), String> {
    if value.starts_with("postgres://") || value.starts_with("postgresql://") {
        Ok(())
    } else {
        Err(format!("{name} must be a Postgres URL"))
    }
}

fn duration_ms(
    values: &HashMap<&str, &str>,
    name: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<Duration, String> {
    number(values, name, default, min, max).map(Duration::from_millis)
}

fn duration_hours(
    values: &HashMap<&str, &str>,
    name: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<Duration, String> {
    number(values, name, default, min, max)
        .and_then(|hours| {
            hours
                .checked_mul(3_600)
                .ok_or_else(|| format!("{name} is too large"))
        })
        .map(Duration::from_secs)
}

fn number<T>(
    values: &HashMap<&str, &str>,
    name: &str,
    default: T,
    min: T,
    max: T,
) -> Result<T, String>
where
    T: Copy + Ord + std::str::FromStr + std::fmt::Display,
{
    let value = match values.get(name) {
        Some(raw) => raw
            .trim()
            .parse::<T>()
            .map_err(|_| format!("{name} must be an integer"))?,
        None => default,
    };
    if value < min || value > max {
        return Err(format!("{name} must be between {min} and {max}"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::MainQueueConfig;

    #[test]
    fn defaults_match_queue_design() {
        let config = MainQueueConfig::from_env_values(&[(
            "DATABASE_URL",
            "postgres://postgres@localhost/previa",
        )])
        .expect("valid queue config");

        assert_eq!(config.runner_stale_after, Duration::from_millis(15_000));
        assert_eq!(config.job_lease, Duration::from_millis(30_000));
        assert_eq!(config.job_max_attempts, 3);
        assert_eq!(config.retry_backoff_base, Duration::from_millis(1_000));
        assert_eq!(config.retry_backoff_max, Duration::from_millis(30_000));
    }

    #[test]
    fn rejects_sqlite_runtime_database() {
        let error = MainQueueConfig::from_env_values(&[("DATABASE_URL", "sqlite://previa.db")])
            .expect_err("sqlite runtime must be rejected");

        assert!(error.contains("Postgres"));
    }
}
