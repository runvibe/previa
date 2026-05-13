pub fn estimate_runner_count(target_rps: u64, rps_per_runner: u64) -> Result<usize, &'static str> {
    if rps_per_runner == 0 {
        return Err("rps_per_runner must be greater than zero");
    }
    Ok(target_rps.max(1).div_ceil(rps_per_runner) as usize)
}

#[cfg(test)]
mod tests {
    use super::estimate_runner_count;

    #[test]
    fn estimates_runner_count_with_ceiling_division() {
        assert_eq!(estimate_runner_count(50_000, 5_000), Ok(10));
        assert_eq!(estimate_runner_count(50_001, 5_000), Ok(11));
        assert_eq!(estimate_runner_count(1, 5_000), Ok(1));
    }

    #[test]
    fn rejects_zero_rps_per_runner() {
        assert_eq!(
            estimate_runner_count(50_000, 0),
            Err("rps_per_runner must be greater than zero")
        );
    }
}
