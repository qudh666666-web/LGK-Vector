pub mod edit_file;
pub mod find_module;
pub mod find_module_template;
pub mod generate_code;
pub mod get_param_definition;
pub mod inspect_ecuc_containers;
pub mod locate_container;
pub mod manage_errors;

use anyhow::Result;
use serde_json::Value;

pub(crate) fn required_module(request: &Value) -> Result<&str> {
    request
        .get("module")
        .or_else(|| request.get("module_name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "module is required (accepted keys: module or module_name; use the ECUC short name, for example CanTrcv, not a driver package name such as CanTrcv_30_Tja1040)"
            )
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::required_module;

    #[test]
    fn module_name_is_accepted_as_compatibility_alias() {
        assert_eq!(required_module(&json!({"module": "Can"})).unwrap(), "Can");
        assert_eq!(
            required_module(&json!({"module_name": "CanTrcv"})).unwrap(),
            "CanTrcv"
        );
    }

    #[test]
    fn missing_module_error_explains_expected_name() {
        let message = required_module(&json!({})).unwrap_err().to_string();
        assert!(message.contains("ECUC short name"));
        assert!(message.contains("module_name"));
    }
}
