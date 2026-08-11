use anyhow::Result;
use serde::Serialize;
use serde_json::{json, Value};

use crate::project::SessionConfig;
use crate::vector::param_definition_index::{DefinitionInfo, ParamDefinitionIndex};
use crate::vector::template_index::TemplateIndex;

/// A compact module-outline node.  The default template query deliberately
/// returns names and hierarchy only; callers use get_param_definition for the
/// description/range of the few definitions they actually need.
#[derive(Debug, Serialize)]
struct TemplateNode {
    name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    subcontainers: Vec<TemplateNode>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    parameters: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    references: Vec<String>,
}

pub fn execute(config: &SessionConfig, request: &Value) -> Result<Value> {
    let module = super::required_module(request)?;
    let template = TemplateIndex::load(config, module)?;
    let definitions = ParamDefinitionIndex::load(&template)?;

    let containers = definitions
        .all()
        .iter()
        .filter(|item| item.group == "containers")
        .count();
    let parameters = definitions
        .all()
        .iter()
        .filter(|item| item.group == "parameters")
        .count();
    let references = definitions
        .all()
        .iter()
        .filter(|item| item.group == "references")
        .count();

    let mut result = json!({
        "module": template.module,
        "definition_ref": template.definition_ref,
        "template_path": template.path,
        "counts": {
            "containers": containers,
            "parameters": parameters,
            "references": references,
        },
        "containers": root_containers(definitions.all()),
        "parameters": direct_members(definitions.all(), "parameters", None),
        "references": direct_members(definitions.all(), "references", None),
    });

    // Keep an explicit diagnostic escape hatch for maintainers.  The normal
    // agent workflow stays compact, while details=true preserves access to the
    // complete definition metadata without changing get_param_definition.
    if request.get("details").and_then(Value::as_bool) == Some(true) {
        result["definitions"] = serde_json::to_value(definitions.all())?;
    }
    Ok(result)
}

fn root_containers(definitions: &[DefinitionInfo]) -> Vec<TemplateNode> {
    definitions
        .iter()
        .filter(|item| item.group == "containers" && parent_container(definitions, item).is_none())
        .map(|item| build_container(definitions, item))
        .collect()
}

fn build_container(definitions: &[DefinitionInfo], container: &DefinitionInfo) -> TemplateNode {
    let parent = Some(container.definition_ref.as_str());
    let subcontainers = definitions
        .iter()
        .filter(|item| {
            item.group == "containers"
                && parent_container(definitions, item)
                    .is_some_and(|candidate| candidate.definition_ref == container.definition_ref)
        })
        .map(|item| build_container(definitions, item))
        .collect();

    TemplateNode {
        name: container.name.clone(),
        subcontainers,
        parameters: direct_members(definitions, "parameters", parent),
        references: direct_members(definitions, "references", parent),
    }
}

fn direct_members(
    definitions: &[DefinitionInfo],
    group: &str,
    parent_ref: Option<&str>,
) -> Vec<String> {
    definitions
        .iter()
        .filter(|item| {
            item.group == group
                && parent_container(definitions, item).map(|parent| parent.definition_ref.as_str())
                    == parent_ref
        })
        .map(|item| item.name.clone())
        .collect()
}

fn parent_container<'a>(
    definitions: &'a [DefinitionInfo],
    item: &DefinitionInfo,
) -> Option<&'a DefinitionInfo> {
    definitions
        .iter()
        .filter(|candidate| {
            candidate.group == "containers"
                && candidate.definition_ref.len() < item.definition_ref.len()
                && item.definition_ref.starts_with(&candidate.definition_ref)
                && item.definition_ref.as_bytes().get(candidate.definition_ref.len()) == Some(&b'/')
        })
        .max_by_key(|candidate| candidate.definition_ref.len())
}
