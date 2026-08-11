use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;
use xmltree::{Element, XMLNode};

use crate::project::SessionConfig;

// DPA 中一个 Module 条目与它实际 ECUC 文件之间的映射。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModuleInfo {
    pub module: String,
    pub name: String,
    pub config_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ModuleIndex {
    modules: Vec<ModuleInfo>,
}

impl ModuleIndex {
    pub fn load(config: &SessionConfig) -> Result<Self> {
        // DPA 只负责回答“模块配置文件在哪”；它不是 BSWMD 参数模板。
        let dpa_path = config.dpa_file()?;
        let root = Element::parse(
            File::open(&dpa_path)
                .with_context(|| format!("cannot open dpa file: {}", dpa_path.display()))?,
        )
        .with_context(|| format!("failed to parse dpa file: {}", dpa_path.display()))?;
        let splitter = child(&root, "EcucSplitter")
            .ok_or_else(|| anyhow::anyhow!("EcucSplitter not found in {}", dpa_path.display()))?;

        let mut modules = Vec::new();
        // 一个 Splitter 可能包含多个 Module，它们共享同一份 ECUC ARXML 文件。
        for node in &splitter.children {
            let XMLNode::Element(splitter_entry) = node else {
                continue;
            };
            if splitter_entry.name != "Splitter" {
                continue;
            }
            let Some(file) = splitter_entry.attributes.get("File") else {
                continue;
            };
            for module in child_elements(splitter_entry, "Module") {
                let Some(name) = module.attributes.get("Name") else {
                    continue;
                };
                modules.push(ModuleInfo {
                    module: name.clone(),
                    name: name.clone(),
                    config_path: normalize_relative(&config.project_path, file),
                });
            }
        }

        if modules.is_empty() {
            bail!(
                "no modules found under EcucSplitter/Splitter/Module in {}",
                dpa_path.display()
            );
        }
        modules.sort_by(|left, right| left.module.cmp(&right.module));
        Ok(Self { modules })
    }

    pub fn all(&self) -> &[ModuleInfo] {
        &self.modules
    }

    pub fn find(&self, module: &str) -> Result<&ModuleInfo> {
        // 对外部请求使用大小写无关匹配，避免 Com/com 造成无意义失败。
        self.modules
            .iter()
            .find(|item| item.module.eq_ignore_ascii_case(module))
            .ok_or_else(|| anyhow::anyhow!("module not found in current project: {module}"))
    }
}

fn child<'a>(element: &'a Element, name: &str) -> Option<&'a Element> {
    child_elements(element, name).into_iter().next()
}

fn child_elements<'a>(element: &'a Element, name: &str) -> Vec<&'a Element> {
    element
        .children
        .iter()
        .filter_map(|node| match node {
            XMLNode::Element(child) if child.name == name => Some(child),
            _ => None,
        })
        .collect()
}

fn normalize_relative(project: &Path, raw: &str) -> PathBuf {
    let normalized = raw.replace('/', "\\");
    let relative = normalized
        .strip_prefix(".\\")
        .unwrap_or(&normalized)
        .to_string();
    project.join(relative)
}
