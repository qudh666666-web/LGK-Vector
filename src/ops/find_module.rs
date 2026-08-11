use anyhow::{bail, Result};
use serde_json::{json, Value};

use crate::project::SessionConfig;
use crate::vector::module_index::ModuleIndex;
use crate::vector::template_index::read_module_definition_ref;

pub fn execute(config: &SessionConfig, request: &Value) -> Result<Value> {
    // 模块名来自 DPA 的 Module Name；它不是文件路径，也不是 definition_ref。
    let module = super::required_module(request)?;
    let index = ModuleIndex::load(config)?;
    if module.eq_ignore_ascii_case("all") {
        return Ok(serde_json::to_value(index.all())?);
    }
    if module.contains('/') || module.contains('\\') {
        bail!("module must be a short module name");
    }
    let found = index.find(module)?;
    // 返回真实 definition_ref，后续定位或生成应复用它而不是手写供应商前缀。
    let definition_ref = read_module_definition_ref(&found.config_path, &found.module)?;
    Ok(json!({
        "module": found.module,
        "name": found.name,
        "configPath": found.config_path,
        "definition_ref": definition_ref,
    }))
}
