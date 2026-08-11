use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;

use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::Serialize;
use serde_json::{to_value, Value};
use xmltree::{Element, XMLNode};

use crate::project::SessionConfig;
use crate::vector::module_index::ModuleIndex;
use crate::vector::search::{child, child_text};

/// A compact, read-only view of one configured ECUC container.
///
/// This is deliberately configuration data, not generated C code. It lets a
/// caller inspect a locked/open DaVinci project without editing ARXML.
#[derive(Debug, Serialize)]
struct ContainerSnapshot {
    module: String,
    config_path: String,
    container_path: String,
    short_name: String,
    definition_ref: String,
    values: BTreeMap<String, String>,
}

pub fn execute(config: &SessionConfig, request: &Value) -> Result<Value> {
    let module = super::required_module(request)?;
    let definition_ref = optional_string(request, "definition_ref");
    let container = optional_string(request, "container");
    if definition_ref.is_none() && container.is_none() {
        bail!("definition_ref or container is required");
    }

    let name_pattern = optional_string(request, "short_name_regex").unwrap_or(".*");
    let name_regex = Regex::new(name_pattern)
        .with_context(|| format!("invalid short_name_regex: {name_pattern}"))?;
    let requested_params = requested_params(request)?;

    let modules = ModuleIndex::load(config)?;
    let module_info = modules.find(module)?;
    let root = Element::parse(File::open(&module_info.config_path).with_context(|| {
        format!(
            "cannot open ECUC configuration: {}",
            module_info.config_path.display()
        )
    })?)
    .with_context(|| {
        format!(
            "failed to parse ECUC configuration: {}",
            module_info.config_path.display()
        )
    })?;

    let mut snapshots = Vec::new();
    visit_for_top_level_containers(
        &root,
        &module_info.module,
        &module_info.config_path.display().to_string(),
        definition_ref,
        container,
        &name_regex,
        requested_params.as_ref(),
        &mut snapshots,
    );
    to_value(snapshots).context("serialize ECUC container snapshots")
}

#[allow(clippy::too_many_arguments)]
fn visit_for_top_level_containers(
    element: &Element,
    module: &str,
    config_path: &str,
    definition_ref: Option<&str>,
    container: Option<&str>,
    name_regex: &Regex,
    requested_params: Option<&BTreeSet<String>>,
    snapshots: &mut Vec<ContainerSnapshot>,
) {
    if element.name == "ECUC-CONTAINER-VALUE" {
        collect_container(
            element,
            &[],
            module,
            config_path,
            definition_ref,
            container,
            name_regex,
            requested_params,
            snapshots,
        );
        return;
    }
    for node in &element.children {
        if let XMLNode::Element(child) = node {
            visit_for_top_level_containers(
                child,
                module,
                config_path,
                definition_ref,
                container,
                name_regex,
                requested_params,
                snapshots,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_container(
    element: &Element,
    parent_names: &[String],
    module: &str,
    config_path: &str,
    expected_definition_ref: Option<&str>,
    expected_container: Option<&str>,
    name_regex: &Regex,
    requested_params: Option<&BTreeSet<String>>,
    snapshots: &mut Vec<ContainerSnapshot>,
) {
    let short_name = child_text(element, "SHORT-NAME").unwrap_or_default();
    let definition_ref = child_text(element, "DEFINITION-REF").unwrap_or_default();
    let mut names = parent_names.to_vec();
    if !short_name.is_empty() {
        names.push(short_name.clone());
    }

    let definition_matches = expected_definition_ref
        .is_some_and(|expected| definition_ref.eq_ignore_ascii_case(expected));
    let container_matches = expected_container.is_some_and(|expected| {
        definition_ref
            .rsplit('/')
            .next()
            .is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
    });
    if (definition_matches || container_matches) && name_regex.is_match(&short_name) {
        snapshots.push(ContainerSnapshot {
            module: module.to_string(),
            config_path: config_path.to_string(),
            container_path: format!("/{}", names.join("/")),
            short_name: short_name.clone(),
            definition_ref: definition_ref.clone(),
            values: collect_values(element, requested_params),
        });
    }

    if let Some(sub_containers) = child(element, "SUB-CONTAINERS") {
        for node in &sub_containers.children {
            if let XMLNode::Element(child) = node {
                if child.name == "ECUC-CONTAINER-VALUE" {
                    collect_container(
                        child,
                        &names,
                        module,
                        config_path,
                        expected_definition_ref,
                        expected_container,
                        name_regex,
                        requested_params,
                        snapshots,
                    );
                }
            }
        }
    }
}

fn collect_values(
    container: &Element,
    requested_params: Option<&BTreeSet<String>>,
) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for group_name in ["PARAMETER-VALUES", "REFERENCE-VALUES"] {
        let Some(group) = child(container, group_name) else {
            continue;
        };
        for node in &group.children {
            let XMLNode::Element(value_node) = node else {
                continue;
            };
            let Some(definition_ref) = child_text(value_node, "DEFINITION-REF") else {
                continue;
            };
            let Some(name) = definition_ref.rsplit('/').next() else {
                continue;
            };
            if requested_params.is_some_and(|requested| !requested.contains(name)) {
                continue;
            }
            let value =
                child_text(value_node, "VALUE").or_else(|| child_text(value_node, "VALUE-REF"));
            if let Some(value) = value {
                values.insert(name.to_string(), value);
            }
        }
    }
    values
}

fn requested_params(request: &Value) -> Result<Option<BTreeSet<String>>> {
    let Some(value) = request.get("params") else {
        return Ok(None);
    };
    let values = match value {
        Value::String(raw) => raw.split(',').map(str::trim).collect::<Vec<_>>(),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::trim)
                    .ok_or_else(|| anyhow::anyhow!("params array must contain only strings"))
            })
            .collect::<Result<Vec<_>>>()?,
        _ => bail!("params must be a comma-separated string or string array"),
    };
    let values = values
        .into_iter()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    Ok(Some(values))
}

fn optional_string<'a>(request: &'a Value, name: &str) -> Option<&'a str> {
    request
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}
