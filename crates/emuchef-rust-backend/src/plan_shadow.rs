//! Dev-only Rust planner shadow command support.
//!
//! This module intentionally exposes only a process-style helper for the
//! `emuchef-plan-shadow` binary. It reuses the private Rust planner path for
//! manual migration inspection and does not add protocol, Tauri, executor, or
//! Python CLI routing.

use std::path::PathBuf;

use serde_json::Value;

use crate::model::OrderedMap;
use crate::planner::{plan_execution, PlannerInput, PlanningStatus};
use crate::ProcessOutput;

const USAGE: &str = "usage: emuchef-plan-shadow --authored-root <path> --device-plan <id> [--bind <recipe_ref>/<input_id>=<value>]...\n";

#[derive(Debug, PartialEq)]
struct ShadowConfig {
    authored_root: PathBuf,
    device_plan: String,
    input_bindings: OrderedMap<Value>,
}

/// Run the dev-only planner shadow command with already-split process args.
///
/// Planner results are emitted as pretty JSON on stdout, including planner
/// validation failures such as missing required bindings. Argument and authored
/// load failures are process-level errors and write stable text to stderr.
pub fn run(args: &[String]) -> ProcessOutput {
    let config = match parse_args(args) {
        Ok(config) => config,
        Err(ShadowArgError::Help) => {
            return ProcessOutput {
                exit_code: 0,
                stdout: USAGE.to_string(),
                stderr: String::new(),
            };
        }
        Err(ShadowArgError::Usage(message)) => {
            return ProcessOutput {
                exit_code: 2,
                stdout: String::new(),
                stderr: format!("{message}\n{USAGE}"),
            };
        }
    };

    let input = match PlannerInput::from_authored_device_plan(
        &config.authored_root,
        &config.device_plan,
        format!("plan.shadow.{}.001", config.device_plan),
        config.input_bindings,
    ) {
        Ok(input) => input,
        Err(error) => {
            return ProcessOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: format!("Error: {}: {error}\n", error.code()),
            };
        }
    };

    let result = plan_execution(input);
    let exit_code = if matches!(result.status, PlanningStatus::Success) {
        0
    } else {
        1
    };
    ProcessOutput {
        exit_code,
        stdout: format!(
            "{}\n",
            serde_json::to_string_pretty(&result)
                .expect("serializing shadow planning result should not fail")
        ),
        stderr: String::new(),
    }
}

#[derive(Debug, PartialEq)]
enum ShadowArgError {
    Help,
    Usage(String),
}

fn parse_args(args: &[String]) -> Result<ShadowConfig, ShadowArgError> {
    let mut authored_root: Option<PathBuf> = None;
    let mut device_plan: Option<String> = None;
    let mut raw_bindings: Vec<String> = Vec::new();
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--authored-root" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error("--authored-root requires one argument");
                };
                authored_root = Some(PathBuf::from(value));
            }
            "--device-plan" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error("--device-plan requires one argument");
                };
                device_plan = Some(value.clone());
            }
            "--bind" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error("--bind requires one argument");
                };
                raw_bindings.push(value.clone());
            }
            "-h" | "--help" => return Err(ShadowArgError::Help),
            value if value.starts_with('-') => {
                return usage_error(&format!("unrecognized argument: {value}"));
            }
            value => {
                return usage_error(&format!("unrecognized argument: {value}"));
            }
        }
        index += 1;
    }

    let missing = [
        ("--authored-root", authored_root.is_none()),
        ("--device-plan", device_plan.is_none()),
    ]
    .into_iter()
    .filter_map(|(name, is_missing)| is_missing.then_some(name))
    .collect::<Vec<_>>();
    if !missing.is_empty() {
        return usage_error(&format!(
            "the following arguments are required: {}",
            missing.join(", ")
        ));
    }

    Ok(ShadowConfig {
        authored_root: authored_root.expect("missing authored_root was checked"),
        device_plan: device_plan.expect("missing device_plan was checked"),
        input_bindings: parse_bindings(&raw_bindings)?,
    })
}

fn parse_bindings(raw_bindings: &[String]) -> Result<OrderedMap<Value>, ShadowArgError> {
    let mut grouped: OrderedMap<Vec<String>> = OrderedMap::new();
    for raw_binding in raw_bindings {
        let (binding_ref, raw_value) = parse_binding(raw_binding)?;
        grouped
            .entry(binding_ref)
            .or_default()
            .push(raw_value.to_string());
    }

    Ok(grouped
        .into_iter()
        .map(|(binding_ref, raw_values)| {
            let value = if raw_values.len() == 1 {
                Value::String(
                    raw_values
                        .into_iter()
                        .next()
                        .expect("single binding value should exist"),
                )
            } else {
                Value::Array(raw_values.into_iter().map(Value::String).collect())
            };
            (binding_ref, value)
        })
        .collect())
}

fn parse_binding(raw_binding: &str) -> Result<(String, &str), ShadowArgError> {
    let Some((binding_ref, raw_value)) = raw_binding.split_once('=') else {
        return invalid_bind(raw_binding);
    };
    let Some((recipe_ref, input_id)) = binding_ref.split_once('/') else {
        return invalid_bind(raw_binding);
    };
    if recipe_ref.is_empty() || input_id.is_empty() || input_id.contains('/') {
        return invalid_bind(raw_binding);
    }
    Ok((binding_ref.to_string(), raw_value))
}

fn usage_error<T>(message: &str) -> Result<T, ShadowArgError> {
    Err(ShadowArgError::Usage(format!(
        "emuchef-plan-shadow: error: {message}"
    )))
}

fn invalid_bind<T>(raw_binding: &str) -> Result<T, ShadowArgError> {
    usage_error(&format!(
        "Invalid --bind value: '{raw_binding}'. Expected <recipe_ref>/<input_id>=<value>."
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parse_bindings_matches_python_repeated_ref_grouping() {
        let bindings = parse_bindings(&strings(&[
            "recipe.one/input=/tmp/one",
            "recipe.two/input=/tmp/two",
            "recipe.one/input=/tmp/three",
        ]))
        .expect("bindings should parse");

        assert_eq!(
            bindings["recipe.one/input"],
            json!(["/tmp/one", "/tmp/three"])
        );
        assert_eq!(bindings["recipe.two/input"], json!("/tmp/two"));
        assert_eq!(
            bindings.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["recipe.one/input", "recipe.two/input"]
        );
    }

    #[test]
    fn parse_bindings_rejects_malformed_refs() {
        for raw in [
            "missing-equals",
            "/input=/tmp/value",
            "recipe/=/tmp/value",
            "recipe/input/extra=/tmp/value",
        ] {
            let error = parse_bindings(&strings(&[raw])).expect_err("binding should fail");
            assert!(
                matches!(error, ShadowArgError::Usage(message) if message.contains("Invalid --bind value"))
            );
        }
    }
}
