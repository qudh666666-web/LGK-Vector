use std::collections::BTreeMap;

use anyhow::Result;
use serde::Serialize;
use serde_json::{json, Value};
use xmltree::{Element, XMLNode};

use crate::vector::search::{child, child_text, is_definition};
use crate::vector::template_index::TemplateIndex;

// 暴露给 JSON 调用方的一个 ECUC 定义：容器、参数或引用。
#[derive(Debug, Clone, Serialize)]
pub struct DefinitionInfo {
    pub name: String,
    pub definition_ref: String,
    pub group: String,
    pub description: String,
    pub value_tag: String,
    pub dest: String,
    pub range: Value,
    pub ref_target: Option<String>,
}

#[derive(Debug)]
pub struct ParamDefinitionIndex {
    definitions: Vec<DefinitionInfo>,
}

impl ParamDefinitionIndex {
    pub fn load(template: &TemplateIndex) -> Result<Self> {
        // 从已经精确匹配的模块模板根开始递归，构建所有完整 definition_ref。
        let module = template.module_definition()?;
        let mut definitions = Vec::new();
        collect_definitions(module, &template.definition_ref, &mut definitions, true);
        definitions.sort_by(|left, right| left.definition_ref.cmp(&right.definition_ref));
        Ok(Self { definitions })
    }

    pub fn all(&self) -> &[DefinitionInfo] {
        &self.definitions
    }

    pub fn find_many<'a>(&'a self, names: &[&str]) -> Vec<&'a DefinitionInfo> {
        // 调用方既可传短名，也可传完整 definition_ref。
        self.definitions
            .iter()
            .filter(|item| {
                names.iter().any(|name| {
                    item.name.eq_ignore_ascii_case(name)
                        || item.definition_ref.eq_ignore_ascii_case(name)
                })
            })
            .collect()
    }
}

fn collect_definitions(
    element: &Element,
    parent_ref: &str,
    output: &mut Vec<DefinitionInfo>,
    module_root: bool,
) {
    // definition_ref 由父节点路径逐层拼出，和 ARXML 的嵌套结构保持一致。
    let recognized = !module_root && is_definition(element);
    let mut current_ref = parent_ref.to_string();

    if recognized {
        if let Some(name) = child_text(element, "SHORT-NAME") {
            current_ref = format!("{}/{}", parent_ref.trim_end_matches('/'), name);
            output.push(DefinitionInfo {
                name,
                definition_ref: current_ref.clone(),
                group: definition_group(element),
                description: description(element),
                value_tag: value_tag(&element.name).to_string(),
                dest: element.name.clone(),
                range: range(element),
                ref_target: first_metadata_text(
                    element,
                    &[
                        "DESTINATION-TYPE",
                        "DESTINATION-URI",
                        "DESTINATION-URI-DEF-REF",
                    ],
                ),
            });
        }
    }

    for node in &element.children {
        if let XMLNode::Element(child_element) = node {
            collect_definitions(child_element, &current_ref, output, false);
        }
    }
}

fn definition_group(element: &Element) -> String {
    if is_container_definition(&element.name) {
        "containers"
    } else if element.name.ends_with("-REFERENCE-DEF") {
        "references"
    } else {
        "parameters"
    }
    .to_string()
}

fn value_tag(dest: &str) -> &'static str {
    // 返回编辑 ECUC 时应创建/查找的值节点类型，而不是 C 语言类型。
    if is_container_definition(dest) {
        "ECUC-CONTAINER-VALUE"
    } else if dest.ends_with("-REFERENCE-DEF") {
        "ECUC-REFERENCE-VALUE"
    } else if dest.contains("BOOLEAN") || dest.contains("INTEGER") || dest.contains("FLOAT") {
        "ECUC-NUMERICAL-PARAM-VALUE"
    } else {
        "ECUC-TEXTUAL-PARAM-VALUE"
    }
}

fn is_container_definition(dest: &str) -> bool {
    matches!(
        dest,
        "ECUC-PARAM-CONF-CONTAINER-DEF" | "ECUC-CHOICE-CONTAINER-DEF"
    )
}

fn description(element: &Element) -> String {
    let Some(desc) = child(element, "DESC") else {
        return String::new();
    };
    let mut parts = Vec::new();
    collect_text(desc, &mut parts);
    parts
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn collect_text(element: &Element, output: &mut Vec<String>) {
    if let Some(text) = element.get_text() {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            output.push(trimmed.to_string());
        }
    }
    for node in &element.children {
        if let XMLNode::Element(child_element) = node {
            collect_text(child_element, output);
        }
    }
}

fn range(element: &Element) -> Value {
    let mut values = BTreeMap::new();
    for (json_name, xml_name) in [
        ("default", "DEFAULT-VALUE"),
        ("min", "MIN"),
        ("max", "MAX"),
        ("lower_multiplicity", "LOWER-MULTIPLICITY"),
        ("upper_multiplicity", "UPPER-MULTIPLICITY"),
        ("upper_multiplicity_infinite", "UPPER-MULTIPLICITY-INFINITE"),
    ] {
        if let Some(value) = first_metadata_text(element, &[xml_name]) {
            values.insert(json_name, value);
        }
    }
    json!(values)
}

fn first_metadata_text(element: &Element, names: &[&str]) -> Option<String> {
    for node in &element.children {
        let XMLNode::Element(child_element) = node else {
            continue;
        };
        if names.iter().any(|name| child_element.name == *name) {
            if let Some(value) = child_element
                .get_text()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                return Some(value);
            }
        }
        // Metadata may be wrapped, but a nested ECUC definition belongs to a
        // child container/parameter and must never leak into its parent.
        if !is_definition(child_element) {
            if let Some(value) = first_metadata_text(child_element, names) {
                return Some(value);
            }
        }
    }
    None
}
