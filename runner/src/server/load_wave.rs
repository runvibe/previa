use crate::server::models::{LoadInterpolation, LoadPoint, LoadProfile};

pub fn validate_load_profile(profile: &LoadProfile) -> Result<(), String> {
    if profile.points.len() < 2 {
        return Err("load.points must contain at least two points".to_owned());
    }
    if profile.points[0].at_ms != 0 {
        return Err("load.points[0].atMs must be 0".to_owned());
    }
    if profile.runner_max_rps <= 0.0 {
        return Err("load.runnerMaxRps must be positive".to_owned());
    }
    if profile.max_in_flight == 0 {
        return Err("load.maxInFlight must be positive".to_owned());
    }

    for point in &profile.points {
        if !(0.0..=100.0).contains(&point.intensity) {
            return Err("load.points intensity must be between 0 and 100".to_owned());
        }
    }

    for pair in profile.points.windows(2) {
        if pair[1].at_ms <= pair[0].at_ms {
            return Err("load.points must be strictly increasing by atMs".to_owned());
        }
    }

    Ok(())
}

pub fn calculate_tick_ms(profile: &LoadProfile) -> u64 {
    let min_interval = profile
        .points
        .windows(2)
        .map(|pair| pair[1].at_ms.saturating_sub(pair[0].at_ms))
        .filter(|interval| *interval > 0)
        .min()
        .unwrap_or(10_000);

    (min_interval / 10).clamp(100, 1000)
}

pub fn calculate_dispatch_tick_ms(profile: &LoadProfile) -> u64 {
    calculate_tick_ms(profile).min(100)
}

pub fn sample_intensity(profile: &LoadProfile, elapsed_ms: u64) -> f64 {
    let last = profile
        .points
        .last()
        .expect("validated profile must contain points");
    if elapsed_ms >= last.at_ms {
        return last.intensity;
    }

    let (start, end) = find_segment(&profile.points, elapsed_ms);
    if elapsed_ms >= end.at_ms {
        return end.intensity;
    }

    match profile.interpolation {
        LoadInterpolation::Step => start.intensity,
        LoadInterpolation::Linear => interpolate_linear(start, end, elapsed_ms),
        LoadInterpolation::Smooth => {
            let raw_t = segment_t(start, end, elapsed_ms);
            let smooth_t = raw_t * raw_t * (3.0 - 2.0 * raw_t);
            start.intensity + (end.intensity - start.intensity) * smooth_t
        }
    }
}

pub fn local_rps_limit(profile: &LoadProfile, elapsed_ms: u64) -> f64 {
    profile.runner_max_rps * sample_intensity(profile, elapsed_ms) / 100.0
}

pub fn timeline_end_ms(profile: &LoadProfile) -> u64 {
    profile
        .points
        .last()
        .map(|point| point.at_ms)
        .unwrap_or_default()
}

fn find_segment(points: &[LoadPoint], elapsed_ms: u64) -> (&LoadPoint, &LoadPoint) {
    points
        .windows(2)
        .find(|pair| elapsed_ms >= pair[0].at_ms && elapsed_ms < pair[1].at_ms)
        .map(|pair| (&pair[0], &pair[1]))
        .unwrap_or_else(|| (&points[points.len() - 2], &points[points.len() - 1]))
}

fn interpolate_linear(start: &LoadPoint, end: &LoadPoint, elapsed_ms: u64) -> f64 {
    let t = segment_t(start, end, elapsed_ms);
    start.intensity + (end.intensity - start.intensity) * t
}

fn segment_t(start: &LoadPoint, end: &LoadPoint, elapsed_ms: u64) -> f64 {
    let span = end.at_ms.saturating_sub(start.at_ms).max(1) as f64;
    let offset = elapsed_ms.saturating_sub(start.at_ms) as f64;
    (offset / span).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use crate::server::models::{LoadInterpolation, LoadPoint, LoadProfile};

    #[test]
    fn accepts_valid_wave_profile() {
        let profile = LoadProfile {
            points: vec![
                LoadPoint {
                    at_ms: 0,
                    intensity: 10.0,
                },
                LoadPoint {
                    at_ms: 60_000,
                    intensity: 80.0,
                },
            ],
            interpolation: LoadInterpolation::Smooth,
            runner_max_rps: 1000.0,
            max_in_flight: 200,
            grace_period_ms: 30_000,
        };

        assert!(super::validate_load_profile(&profile).is_ok());
    }

    #[test]
    fn rejects_wave_without_zero_start() {
        let profile = LoadProfile {
            points: vec![
                LoadPoint {
                    at_ms: 100,
                    intensity: 10.0,
                },
                LoadPoint {
                    at_ms: 60_000,
                    intensity: 80.0,
                },
            ],
            interpolation: LoadInterpolation::Smooth,
            runner_max_rps: 1000.0,
            max_in_flight: 200,
            grace_period_ms: 30_000,
        };

        assert_eq!(
            super::validate_load_profile(&profile).unwrap_err(),
            "load.points[0].atMs must be 0"
        );
    }

    #[test]
    fn rejects_non_increasing_points() {
        let profile = LoadProfile {
            points: vec![
                LoadPoint {
                    at_ms: 0,
                    intensity: 10.0,
                },
                LoadPoint {
                    at_ms: 0,
                    intensity: 80.0,
                },
            ],
            interpolation: LoadInterpolation::Smooth,
            runner_max_rps: 1000.0,
            max_in_flight: 200,
            grace_period_ms: 30_000,
        };

        assert_eq!(
            super::validate_load_profile(&profile).unwrap_err(),
            "load.points must be strictly increasing by atMs"
        );
    }

    #[test]
    fn rejects_out_of_range_intensity() {
        let profile = LoadProfile {
            points: vec![
                LoadPoint {
                    at_ms: 0,
                    intensity: 10.0,
                },
                LoadPoint {
                    at_ms: 60_000,
                    intensity: 120.0,
                },
            ],
            interpolation: LoadInterpolation::Smooth,
            runner_max_rps: 1000.0,
            max_in_flight: 200,
            grace_period_ms: 30_000,
        };

        assert_eq!(
            super::validate_load_profile(&profile).unwrap_err(),
            "load.points intensity must be between 0 and 100"
        );
    }

    #[test]
    fn calculates_dynamic_tick_with_minimum_and_maximum() {
        let long_profile = LoadProfile {
            points: vec![
                LoadPoint {
                    at_ms: 0,
                    intensity: 10.0,
                },
                LoadPoint {
                    at_ms: 60_000,
                    intensity: 80.0,
                },
            ],
            interpolation: LoadInterpolation::Smooth,
            runner_max_rps: 1000.0,
            max_in_flight: 200,
            grace_period_ms: 30_000,
        };
        assert_eq!(super::calculate_tick_ms(&long_profile), 1000);

        let short_profile = LoadProfile {
            points: vec![
                LoadPoint {
                    at_ms: 0,
                    intensity: 10.0,
                },
                LoadPoint {
                    at_ms: 500,
                    intensity: 80.0,
                },
            ],
            interpolation: LoadInterpolation::Smooth,
            runner_max_rps: 1000.0,
            max_in_flight: 200,
            grace_period_ms: 30_000,
        };
        assert_eq!(super::calculate_tick_ms(&short_profile), 100);
    }

    #[test]
    fn dispatch_tick_uses_fine_grained_100ms_cadence() {
        let long_profile = LoadProfile {
            points: vec![
                LoadPoint {
                    at_ms: 0,
                    intensity: 10.0,
                },
                LoadPoint {
                    at_ms: 120_000,
                    intensity: 80.0,
                },
            ],
            interpolation: LoadInterpolation::Smooth,
            runner_max_rps: 1000.0,
            max_in_flight: 200,
            grace_period_ms: 30_000,
        };

        assert_eq!(super::calculate_tick_ms(&long_profile), 1000);
        assert_eq!(super::calculate_dispatch_tick_ms(&long_profile), 100);
    }

    #[test]
    fn interpolates_linear_values() {
        let profile = LoadProfile {
            points: vec![
                LoadPoint {
                    at_ms: 0,
                    intensity: 10.0,
                },
                LoadPoint {
                    at_ms: 1000,
                    intensity: 90.0,
                },
            ],
            interpolation: LoadInterpolation::Linear,
            runner_max_rps: 1000.0,
            max_in_flight: 200,
            grace_period_ms: 30_000,
        };
        assert_eq!(super::sample_intensity(&profile, 500), 50.0);
    }

    #[test]
    fn interpolates_smoothstep_values() {
        let profile = LoadProfile {
            points: vec![
                LoadPoint {
                    at_ms: 0,
                    intensity: 0.0,
                },
                LoadPoint {
                    at_ms: 1000,
                    intensity: 100.0,
                },
            ],
            interpolation: LoadInterpolation::Smooth,
            runner_max_rps: 1000.0,
            max_in_flight: 200,
            grace_period_ms: 30_000,
        };
        assert!((super::sample_intensity(&profile, 250) - 15.625).abs() < 0.001);
        assert_eq!(super::sample_intensity(&profile, 500), 50.0);
    }

    #[test]
    fn interpolates_step_values() {
        let profile = LoadProfile {
            points: vec![
                LoadPoint {
                    at_ms: 0,
                    intensity: 10.0,
                },
                LoadPoint {
                    at_ms: 1000,
                    intensity: 90.0,
                },
            ],
            interpolation: LoadInterpolation::Step,
            runner_max_rps: 1000.0,
            max_in_flight: 200,
            grace_period_ms: 30_000,
        };
        assert_eq!(super::sample_intensity(&profile, 999), 10.0);
        assert_eq!(super::sample_intensity(&profile, 1000), 90.0);
    }
}
