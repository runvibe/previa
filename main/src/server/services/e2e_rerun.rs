use std::collections::HashMap;

use previa_runner::{Pipeline, StepExecutionResult};

pub fn validate_rerun_context(
    pipeline: &Pipeline,
    start_step_id: &str,
    prior_results: &HashMap<String, StepExecutionResult>,
) -> Result<(), String> {
    let start_index = pipeline
        .steps
        .iter()
        .position(|step| step.id == start_step_id)
        .ok_or_else(|| "startStepId not found in pipeline".to_owned())?;

    for step in pipeline.steps.iter().take(start_index) {
        match prior_results.get(&step.id) {
            Some(result) if result.status != "pending" && result.status != "running" => {}
            Some(_) => {
                return Err(format!(
                    "prior result for step '{}' is not completed",
                    step.id
                ));
            }
            None => {
                return Err(format!("prior result for step '{}' is required", step.id));
            }
        }
    }

    Ok(())
}

pub fn ordered_prior_results(
    pipeline: &Pipeline,
    start_step_id: &str,
    prior_results: &HashMap<String, StepExecutionResult>,
) -> Vec<serde_json::Value> {
    pipeline
        .steps
        .iter()
        .take_while(|step| step.id != start_step_id)
        .filter_map(|step| prior_results.get(&step.id))
        .filter_map(|result| serde_json::to_value(result).ok())
        .collect()
}
