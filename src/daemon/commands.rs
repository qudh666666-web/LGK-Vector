use anyhow::{bail, Result};
use serde_json::{json, Value};

use crate::daemon::client::DaVinciClient;
use crate::daemon::project_update;
use crate::ops;
use crate::project::SessionConfig;
use crate::vector::module_index::ModuleIndex;
use crate::vector::template_index::read_module_definition_ref;

// JSON 请求的总调度器。davinci 为 None 时代表尚未启动 DaVinci。
pub struct CommandDispatcher {
    davinci: Option<DaVinciClient>,
}

impl Default for CommandDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandDispatcher {
    pub fn new() -> Self {
        Self { davinci: None }
    }

    pub fn dispatch_batch(&mut self, config: &SessionConfig, raw: &str) -> Result<Value> {
        // 允许批量只读查询；每个元素仍复用同一个工程上下文。
        let parsed: Value =
            serde_json::from_str(raw).map_err(|error| anyhow::anyhow!("invalid JSON: {error}"))?;
        match &parsed {
            Value::Array(items) => {
                let (_, all_inspect) = validate_batch_shape(&parsed)?;
                // Multi-item batches are read-only, so running a full local
                // query once is enough. Doctor keeps the separate preflight;
                // executing every template scan here twice would penalize large SIPs.
                let results = items
                    .iter()
                    .map(|item| self.dispatch_one(config, item))
                    .collect::<Result<Vec<_>>>()?;
                if all_inspect {
                    let flattened = results
                        .into_iter()
                        .flat_map(|result| match result {
                            Value::Array(items) => items,
                            other => vec![other],
                        })
                        .collect();
                    return Ok(Value::Array(flattened));
                }
                Ok(Value::Array(results))
            }
            Value::Object(_) => self.dispatch_one(config, &parsed),
            _ => bail!("request must be a JSON object or array"),
        }
    }

    /// Validate a request and all of its local prerequisites without editing files,
    /// starting the resident host, or launching DaVinci.
    pub fn validate_batch(config: &SessionConfig, raw: &str) -> Result<Vec<String>> {
        let parsed: Value =
            serde_json::from_str(raw).map_err(|error| anyhow::anyhow!("invalid JSON: {error}"))?;
        let (items, _) = validate_batch_shape(&parsed)?;

        items
            .into_iter()
            .map(|request| Self::validate_one(config, request))
            .collect()
    }

    pub fn shutdown(mut self) -> Result<()> {
        // 只有真的启动过 DaVinci 才需要向它发送正常关闭命令。
        if let Some(client) = self.davinci.take() {
            client.shutdown()?;
        }
        Ok(())
    }

    fn validate_one(config: &SessionConfig, request: &Value) -> Result<String> {
        let func = request
            .get("func")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("func is required"))?;
        match func {
            "find_module" | "find_bsw_module" => {
                ops::find_module::execute(config, request)?;
            }
            "find_module_template" | "get_bsw_module_template" => {
                ops::find_module_template::execute(config, request)?;
            }
            "get_param_definition" | "get_bsw_param_definition" => {
                ops::get_param_definition::execute(config, request)?;
            }
            "inspect_ecuc_containers" => {
                ops::inspect_ecuc_containers::execute(config, request)?;
            }
            "locate_container" => {
                ops::locate_container::execute(config, request)?;
            }
            "edit_file" => ops::edit_file::validate(config, request)?,
            "get_errors_list" => {
                optional_module(request)?;
                validate_davinci_dependencies(config)?;
            }
            "auto_solve_errors" => {
                if request.get("confirmed").and_then(Value::as_bool) != Some(true) {
                    bail!(
                        "auto_solve_errors requires confirmed=true after reviewing a fresh error list"
                    );
                }
                ops::required_module(request)?;
                if request
                    .get("targets")
                    .is_some_and(|value| !value.is_string())
                {
                    bail!("targets must be a string when it is specified");
                }
                validate_davinci_dependencies(config)?;
            }
            "generate_code" => {
                // Compatibility with the established bridge: an omitted
                // module explicitly means full generation.  The Skill still
                // sends a concrete affected module for the normal fast path.
                let module = optional_module(request)?;
                if !module.eq_ignore_ascii_case("all") {
                    let modules = ModuleIndex::load(config)?;
                    let module_info = modules.find(module)?;
                    read_module_definition_ref(&module_info.config_path, &module_info.module)?;
                }
                validate_davinci_dependencies(config)?;
            }
            "update_project" | "import_dbc" => project_update::validate(config, request)?,
            "shutdown_host" => {}
            _ => bail!("unsupported func: {func}"),
        }
        Ok(func.to_string())
    }

    fn dispatch_one(&mut self, config: &SessionConfig, request: &Value) -> Result<Value> {
        let func = request
            .get("func")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("func is required"))?;
        match func {
            // 这些操作只解析本地 DPA/ECUC/BSWMD 文件，不启动 DaVinci。
            "find_module" | "find_bsw_module" => ops::find_module::execute(config, request),
            "find_module_template" | "get_bsw_module_template" => {
                ops::find_module_template::execute(config, request)
            }
            "get_param_definition" | "get_bsw_param_definition" => {
                ops::get_param_definition::execute(config, request)
            }
            "inspect_ecuc_containers" => ops::inspect_ecuc_containers::execute(config, request),
            "locate_container" => ops::locate_container::execute(config, request),
            "edit_file" => {
                // A previous get_errors/generate request may have left DaVinci's
                // in-memory project open. Close that owned session before a
                // disk edit so the stale model cannot later overwrite ARXML.
                if let Some(client) = self.davinci.take() {
                    client.shutdown()?;
                }
                ops::edit_file::execute(config, request)
            }
            "get_errors_list" => {
                // 错误列表来自 DaVinci 的校验模型，不能靠静态 XML 解析替代。
                let module = optional_module(request)?;
                self.with_davinci(config, |client| client.list_errors(module))
            }
            "auto_solve_errors" => {
                if request.get("confirmed").and_then(Value::as_bool) != Some(true) {
                    bail!(
                        "auto_solve_errors requires confirmed=true after reviewing a fresh error list"
                    );
                }
                let module = ops::required_module(request)?;
                let targets = request.get("targets").and_then(Value::as_str);
                let message =
                    self.with_davinci(config, |client| client.solve_errors(module, targets))?;
                Ok(json!({"module": module, "message": message}))
            }
            "generate_code" => {
                // The protocol keeps the established omitted-module => all
                // behavior. Normal Skill calls still name the affected module.
                let module = optional_module(request)?;
                // 单模块生成的关键：从当前工程读取真实定义路径，
                // 而不是把 /MICROSAR/<module> 或某个芯片/SIP 写死。
                let definition_ref = if module.eq_ignore_ascii_case("all") {
                    None
                } else {
                    let modules = ModuleIndex::load(config)?;
                    let module_info = modules.find(module)?;
                    Some(read_module_definition_ref(
                        &module_info.config_path,
                        &module_info.module,
                    )?)
                };
                let message = self.with_davinci(config, |client| {
                    client.generate(module, definition_ref.as_deref())
                })?;
                Ok(json!({
                    "module": module,
                    "definition_ref": definition_ref,
                    "message": message
                }))
            }
            "update_project" | "import_dbc" => {
                // Project Update must own the DPA exclusively. Close an already
                // opened Groovy-backed session before starting the one-shot CLI.
                if let Some(client) = self.davinci.take() {
                    client.shutdown()?;
                }
                project_update::execute(config, request)
            }
            "shutdown_host" => {
                bail!("shutdown_host is handled by the resident host protocol")
            }
            _ => bail!("unsupported func: {func}"),
        }
    }

    fn davinci(&mut self, config: &SessionConfig) -> Result<&mut DaVinciClient> {
        // 惰性启动：普通查询不会付出 DaVinci 冷启动的时间。
        if self.davinci.is_none() {
            self.davinci = Some(DaVinciClient::start(config)?);
        }
        Ok(self.davinci.as_mut().expect("initialized"))
    }

    fn with_davinci<T>(
        &mut self,
        config: &SessionConfig,
        action: impl FnOnce(&mut DaVinciClient) -> Result<T>,
    ) -> Result<T> {
        let result = action(self.davinci(config)?);
        if result.is_err() {
            // A failed transport or malformed daemon response invalidates this
            // client. Dropping it force-stops the owned process, so the next
            // DaVinci-backed request starts a clean session automatically.
            self.davinci.take();
        }
        result
    }
}

fn validate_batch_shape(parsed: &Value) -> Result<(Vec<&Value>, bool)> {
    let items = match parsed {
        Value::Array(items) if items.is_empty() => bail!("request array must not be empty"),
        Value::Array(items) => items.iter().collect::<Vec<_>>(),
        Value::Object(_) => vec![parsed],
        _ => bail!("request must be a JSON object or array"),
    };
    if items.len() > 1
        && items
            .iter()
            .any(|item| request_func(item) == Some("shutdown_host"))
    {
        bail!("shutdown_host must be a standalone request");
    }
    let inspect_count = items
        .iter()
        .filter(|item| request_func(item) == Some("inspect_ecuc_containers"))
        .count();
    if inspect_count != 0 && inspect_count != items.len() {
        bail!("inspect_ecuc_containers cannot be mixed with other functions in one batch");
    }
    if items.len() > 1
        && items
            .iter()
            .any(|item| request_func(item).is_some_and(is_mutating_func))
    {
        bail!(
            "edit_file, auto_solve_errors, generate_code, update_project, and import_dbc must be standalone requests; multi-item batches are read-only"
        );
    }
    Ok((items, inspect_count != 0))
}

fn validate_davinci_dependencies(config: &SessionConfig) -> Result<()> {
    config.dpa_file()?;
    crate::daemon::client::resolve_davinci_command(config)?;
    Ok(())
}

fn request_func(request: &Value) -> Option<&str> {
    request.get("func").and_then(Value::as_str)
}

fn is_mutating_func(func: &str) -> bool {
    matches!(
        func,
        "edit_file" | "auto_solve_errors" | "generate_code" | "update_project" | "import_dbc"
    )
}

fn optional_module(request: &Value) -> Result<&str> {
    let value = request
        .get("module")
        .or_else(|| request.get("module_name"))
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("module must be a non-empty string"))
        })
        .transpose()?;
    Ok(value.unwrap_or("all"))
}
