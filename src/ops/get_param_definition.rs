use anyhow::{bail, Result};
use serde_json::{json, Value};

use crate::project::SessionConfig;
use crate::vector::param_definition_index::ParamDefinitionIndex;
use crate::vector::template_index::TemplateIndex;

pub fn execute(config: &SessionConfig, request: &Value) -> Result<Value> {
    let module = super::required_module(request)?;
    let params = request
        .get("params")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("params is required"))?;
    let names = params
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();

    let template = TemplateIndex::load(config, module)?;
    let index = ParamDefinitionIndex::load(&template)?;
    let matches = index.find_many(&names);
    let missing = names
        .iter()
        .filter(|name| {
            !matches.iter().any(|item| {
                item.name.eq_ignore_ascii_case(name)
                    || item.definition_ref.eq_ignore_ascii_case(name)
            })
        })
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!("parameter definition not found: {}", missing.join(", "));
    }

    Ok(json!({
        "module": module,
        "template_path": template.path,
        "definitions": matches,
    }))
}
