mod assertions;
mod core;
mod execution;
mod template;

use std::collections::HashMap;

use serde_json::Value;

pub use core::types::{
    AssertionResult, Pipeline, PipelineStep, RuntimeEnvGroup, RuntimeSpec, StepAssertion,
    StepExecutionResult, StepRequest, StepResponse,
};
pub use execution::{
    PreparedHttpStep, StartedHttpStep, complete_started_http_step_with_hook, execute_pipeline,
    execute_pipeline_from_step_with_client_runtime_hooks, execute_pipeline_with_client,
    execute_pipeline_with_client_hooks, execute_pipeline_with_client_runtime_request_gate,
    execute_pipeline_with_hooks, execute_pipeline_with_runtime_hooks,
    execute_pipeline_with_runtime_request_gate, execute_pipeline_with_specs_hooks,
    prepare_http_step, send_prepared_http_step, send_prepared_http_step_with_hooks,
    start_prepared_http_step_with_hooks,
};

pub fn render_template_value(
    value: &Value,
    context: &HashMap<String, StepExecutionResult>,
    specs: Option<&[RuntimeSpec]>,
) -> Value {
    template::resolve::resolve_template_variables(value, context, specs, None, None)
}

pub fn render_template_value_with_runtime(
    value: &Value,
    context: &HashMap<String, StepExecutionResult>,
    specs: Option<&[RuntimeSpec]>,
    env_groups: Option<&[RuntimeEnvGroup]>,
    selected_env_group_slug: Option<&str>,
) -> Value {
    template::resolve::resolve_template_variables(
        value,
        context,
        specs,
        env_groups,
        selected_env_group_slug,
    )
}

pub fn render_template_value_simple(value: &Value) -> Value {
    let context = HashMap::<String, StepExecutionResult>::new();
    render_template_value(value, &context, None)
}
