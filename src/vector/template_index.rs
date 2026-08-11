use std::collections::HashMap;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use anyhow::{bail, Context, Result};
use walkdir::WalkDir;
use xmltree::{Element, XMLNode};

use crate::project::SessionConfig;
use crate::vector::module_index::ModuleIndex;
use crate::vector::search::{child_text, descendants};

// 当前模块在 SIP 中的模板文件及其已解析 XML 树。
#[derive(Debug, Clone)]
pub struct TemplateIndex {
    pub module: String,
    pub definition_ref: String,
    pub path: PathBuf,
    pub root: Element,
}

#[derive(Debug, Clone)]
struct CachedTemplate {
    path: PathBuf,
    length: u64,
    modified: Option<SystemTime>,
    root: Element,
}

type TemplateCacheKey = (PathBuf, String);

fn template_cache() -> &'static Mutex<HashMap<TemplateCacheKey, CachedTemplate>> {
    static CACHE: OnceLock<Mutex<HashMap<TemplateCacheKey, CachedTemplate>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

impl TemplateIndex {
    pub fn load(config: &SessionConfig, module: &str) -> Result<Self> {
        // 先从实际 ECUC 读取定义路径，例如 /MICROSAR/Com 或 /Vendor/Nm。
        // 这条路径才是跨芯片、跨资源包的稳定关联键。
        let modules = ModuleIndex::load(config)?;
        let module_info = modules.find(module)?;
        let definition_ref = read_module_definition_ref(&module_info.config_path, module)?;
        let short_name = definition_ref
            .rsplit('/')
            .find(|part| !part.is_empty())
            .ok_or_else(|| anyhow::anyhow!("invalid module definition ref: {definition_ref}"))?;

        let cache_key = (config.tool_path.clone(), definition_ref.clone());
        if let Some(cached) = cached_template(&cache_key) {
            return Ok(Self {
                module: module.to_string(),
                definition_ref,
                path: cached.path,
                root: cached.root,
            });
        }

        // SIP 的布局因版本和供应商而异，因此扫描 tool_path，
        // 再用完整 definition_ref 精确确认，而不是依赖固定目录名。
        let mut candidates = Vec::new();
        for entry in WalkDir::new(&config.tool_path)
            .follow_links(false)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
        {
            let path = entry.path();
            if !path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("arxml"))
            {
                continue;
            }
            let Ok(raw) = fs::read(path) else {
                continue;
            };
            let text = String::from_utf8_lossy(&raw);
            if !text.contains("<ECUC-MODULE-DEF") || !text.contains(short_name) {
                continue;
            }
            let Ok(root) = Element::parse(Cursor::new(&raw)) else {
                continue;
            };
            if module_definition_paths(&root)
                .iter()
                .any(|candidate| candidate == &definition_ref)
            {
                candidates.push((path.to_path_buf(), root));
            }
        }

        match candidates.len() {
            0 => bail!("no template arxml found for module definition: {definition_ref}"),
            1 => {
                let (path, root) = candidates.pop().expect("length checked");
                cache_template(cache_key, &path, &root);
                Ok(Self {
                    module: module.to_string(),
                    definition_ref,
                    path,
                    root,
                })
            }
            _ => bail!(
                "found multiple template arxml files for {}: {}",
                definition_ref,
                candidates
                    .iter()
                    .map(|(path, _)| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    pub fn module_definition(&self) -> Result<&Element> {
        // 后续参数索引只从匹配到的 ECUC-MODULE-DEF 根节点向下遍历。
        let mut module_defs = Vec::new();
        descendants(&self.root, "ECUC-MODULE-DEF", &mut module_defs);
        module_defs
            .into_iter()
            .find(|element| {
                child_text(element, "SHORT-NAME")
                    .is_some_and(|name| self.definition_ref.ends_with(&format!("/{name}")))
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "module definition {} not found in {}",
                    self.definition_ref,
                    self.path.display()
                )
            })
    }
}

fn cached_template(key: &TemplateCacheKey) -> Option<CachedTemplate> {
    let cached = template_cache().lock().ok()?.get(key).cloned()?;
    let metadata = fs::metadata(&cached.path).ok()?;
    if metadata.len() == cached.length && metadata.modified().ok() == cached.modified {
        Some(cached)
    } else {
        if let Ok(mut cache) = template_cache().lock() {
            cache.remove(key);
        }
        None
    }
}

fn cache_template(key: TemplateCacheKey, path: &Path, root: &Element) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    let cached = CachedTemplate {
        path: path.to_path_buf(),
        length: metadata.len(),
        modified: metadata.modified().ok(),
        root: root.clone(),
    };
    if let Ok(mut cache) = template_cache().lock() {
        cache.insert(key, cached);
    }
}

pub fn read_module_definition_ref(config_path: &Path, module: &str) -> Result<String> {
    // 同一配置文件中通常按模块短名匹配；若实例名不同，
    // 再按 DEFINITION-REF 的末段或唯一候选项回退匹配。
    let root = Element::parse(
        fs::File::open(config_path)
            .with_context(|| format!("cannot open module config: {}", config_path.display()))?,
    )
    .with_context(|| {
        format!(
            "failed to parse module config arxml: {}",
            config_path.display()
        )
    })?;
    let mut values = Vec::new();
    descendants(&root, "ECUC-MODULE-CONFIGURATION-VALUES", &mut values);
    let mut candidates = values
        .into_iter()
        .filter_map(|value| {
            let name = child_text(value, "SHORT-NAME")?;
            let definition_ref = child_text(value, "DEFINITION-REF")?;
            Some((name, definition_ref))
        })
        .collect::<Vec<_>>();
    if let Some((_, definition_ref)) = candidates
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(module))
    {
        return Ok(definition_ref.clone());
    }
    if let Some((_, definition_ref)) = candidates.iter().find(|(_, definition_ref)| {
        definition_ref
            .rsplit('/')
            .find(|part| !part.is_empty())
            .is_some_and(|name| name.eq_ignore_ascii_case(module))
    }) {
        return Ok(definition_ref.clone());
    }
    if candidates.len() == 1 {
        return Ok(candidates.pop().expect("length checked").1);
    }
    bail!(
        "cannot identify module {module} in {} from definition refs: {}",
        config_path.display(),
        candidates
            .iter()
            .map(|(_, definition_ref)| definition_ref.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn module_definition_paths(root: &Element) -> Vec<String> {
    // 把 AR-PACKAGE 层级还原为 /Package/.../Module 形式的引用路径。
    let mut output = Vec::new();
    collect_definition_paths(root, &mut Vec::new(), &mut output);
    output
}

fn collect_definition_paths(
    element: &Element,
    packages: &mut Vec<String>,
    output: &mut Vec<String>,
) {
    let is_package = element.name == "AR-PACKAGE";
    if is_package {
        if let Some(name) = child_text(element, "SHORT-NAME") {
            packages.push(name);
        }
    }

    if element.name == "ECUC-MODULE-DEF" {
        if let Some(name) = child_text(element, "SHORT-NAME") {
            output.push(format!("/{}/{}", packages.join("/"), name));
        }
    }

    for node in &element.children {
        if let XMLNode::Element(child) = node {
            collect_definition_paths(child, packages, output);
        }
    }

    if is_package && !packages.is_empty() {
        packages.pop();
    }
}
