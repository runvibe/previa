use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerQueueConfig {
    pub database_url: String,
    pub heartbeat_interval: Duration,
    pub lease_renew_interval: Duration,
    pub poll_interval: Duration,
    pub event_flush_interval: Duration,
    pub event_batch_size: usize,
    pub event_buffer_max: usize,
}

impl RunnerQueueConfig {
    pub fn from_env() -> Result<Self, String> {
        let keys = [
            "PREVIA_QUEUE_DATABASE_URL",
            "PREVIA_QUEUE_HEARTBEAT_INTERVAL_MS",
            "PREVIA_QUEUE_LEASE_RENEW_INTERVAL_MS",
            "PREVIA_QUEUE_POLL_INTERVAL_MS",
            "PREVIA_QUEUE_EVENT_FLUSH_INTERVAL_MS",
            "PREVIA_QUEUE_EVENT_BATCH_SIZE",
            "PREVIA_QUEUE_EVENT_BUFFER_MAX",
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
            .get("PREVIA_QUEUE_DATABASE_URL")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                "PREVIA_QUEUE_DATABASE_URL is required and must use Postgres".to_owned()
            })?;
        if !(database_url.starts_with("postgres://") || database_url.starts_with("postgresql://")) {
            return Err("PREVIA_QUEUE_DATABASE_URL must be a Postgres URL".to_owned());
        }

        let heartbeat_interval = duration_ms(
            &values,
            "PREVIA_QUEUE_HEARTBEAT_INTERVAL_MS",
            5_000,
            1_000,
            60_000,
        )?;
        let lease_renew_interval = duration_ms(
            &values,
            "PREVIA_QUEUE_LEASE_RENEW_INTERVAL_MS",
            10_000,
            1_000,
            300_000,
        )?;
        let poll_interval =
            duration_ms(&values, "PREVIA_QUEUE_POLL_INTERVAL_MS", 1_000, 100, 60_000)?;
        let event_flush_interval = duration_ms(
            &values,
            "PREVIA_QUEUE_EVENT_FLUSH_INTERVAL_MS",
            250,
            10,
            10_000,
        )?;
        let event_batch_size = number(
            &values,
            "PREVIA_QUEUE_EVENT_BATCH_SIZE",
            200_usize,
            1,
            1_000,
        )?;
        let event_buffer_max = number(
            &values,
            "PREVIA_QUEUE_EVENT_BUFFER_MAX",
            5_000_usize,
            200,
            100_000,
        )?;
        if event_buffer_max < event_batch_size {
            return Err(
                "PREVIA_QUEUE_EVENT_BUFFER_MAX buffer must be at least PREVIA_QUEUE_EVENT_BATCH_SIZE"
                    .to_owned(),
            );
        }

        Ok(Self {
            database_url: database_url.to_owned(),
            heartbeat_interval,
            lease_renew_interval,
            poll_interval,
            event_flush_interval,
            event_batch_size,
            event_buffer_max,
        })
    }

    pub fn validate_lease_duration(&self, lease_duration: Duration) -> Result<(), String> {
        if self.lease_renew_interval.saturating_mul(2) >= lease_duration {
            return Err(
                "PREVIA_QUEUE_LEASE_RENEW_INTERVAL_MS must be less than half the claimed lease duration"
                    .to_owned(),
            );
        }
        Ok(())
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
    use super::RunnerQueueConfig;

    #[test]
    fn defaults_match_queue_design() {
        let config = RunnerQueueConfig::from_env_values(&[(
            "PREVIA_QUEUE_DATABASE_URL",
            "postgres://runner@localhost/previa",
        )])
        .expect("valid queue config");

        assert_eq!(config.event_batch_size, 200);
        assert_eq!(config.event_buffer_max, 5_000);
    }

    #[test]
    fn rejects_buffer_smaller_than_batch() {
        let error = RunnerQueueConfig::from_env_values(&[
            (
                "PREVIA_QUEUE_DATABASE_URL",
                "postgres://runner@localhost/previa",
            ),
            ("PREVIA_QUEUE_EVENT_BATCH_SIZE", "500"),
            ("PREVIA_QUEUE_EVENT_BUFFER_MAX", "200"),
        ])
        .expect_err("unsafe buffer must be rejected");

        assert!(error.contains("buffer"));
    }
}
