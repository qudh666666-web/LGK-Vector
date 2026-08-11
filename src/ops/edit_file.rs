use std::cmp::Reverse;
use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::project::SessionConfig;

// 一个基于行号的最小编辑：既支持单行，也支持 "起始行-结束行"。
#[derive(Debug)]
struct Edit {
    start: usize,
    end: usize,
    replacement: String,
}

pub fn execute(config: &SessionConfig, request: &Value) -> Result<Value> {
    // 这里不重排或重新序列化 ARXML，只替换已经确认的行范围，
    // 以尽量保留 DaVinci 原有的格式、注释和无关内容。
    let path = request
        .get("path")
        .and_then(Value::as_str)
        .map(Path::new)
        .ok_or_else(|| anyhow::anyhow!("path is required"))?;
    let path = config.ensure_project_file(path)?;
    let edits = request
        .get("edits")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("edits must be an object"))?;
    if edits.is_empty() {
        bail!("edits must not be empty");
    }

    // 保留 UTF-8 BOM、原始换行符和文件末尾换行，避免纯格式差异污染 Compare。
    let original = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let (has_bom, text_bytes) = original
        .strip_prefix(&[0xEF, 0xBB, 0xBF])
        .map_or((false, original.as_slice()), |bytes| (true, bytes));
    let text = std::str::from_utf8(text_bytes)
        .with_context(|| format!("file is not UTF-8: {}", path.display()))?;
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let normalized = text.replace("\r\n", "\n");
    let trailing_newline = normalized.ends_with('\n');
    let mut lines = normalized
        .strip_suffix('\n')
        .unwrap_or(&normalized)
        .split('\n')
        .map(str::to_string)
        .collect::<Vec<_>>();

    // 先校验所有范围，再从后往前替换，防止前面的插入改变后续行号。
    validate_expected_lines(request, edits, &lines)?;
    let mut parsed = parse_edits(edits, lines.len())?;
    reject_overlaps(&parsed)?;
    parsed.sort_by_key(|edit| Reverse(edit.start));
    for edit in &parsed {
        let replacement = edit
            .replacement
            .replace("\r\n", "\n")
            .split('\n')
            .map(str::to_string)
            .collect::<Vec<_>>();
        lines.splice((edit.start - 1)..edit.end, replacement);
    }

    let mut output = lines.join(newline);
    if trailing_newline {
        output.push_str(newline);
    }
    let mut encoded = Vec::with_capacity(output.len() + usize::from(has_bom) * 3);
    if has_bom {
        encoded.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    encoded.extend_from_slice(output.as_bytes());

    // 先完整写入临时文件并 sync，再复制覆盖原文件，降低中途写坏的风险。
    let temp_path = path.with_extension(format!(
        "{}.lgk-vector.tmp",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("tmp")
    ));
    {
        let mut temp = fs::File::create(&temp_path)
            .with_context(|| format!("failed to create {}", temp_path.display()))?;
        temp.write_all(&encoded)?;
        temp.sync_all()?;
    }
    fs::copy(&temp_path, &path).with_context(|| format!("failed to replace {}", path.display()))?;
    fs::remove_file(&temp_path).ok();

    Ok(json!({
        "path": path,
        "applied_edits": parsed.len(),
        "message": "Edited file",
    }))
}

/// Perform every edit precondition check without writing the target file.
pub fn validate(config: &SessionConfig, request: &Value) -> Result<()> {
    let path = request
        .get("path")
        .and_then(Value::as_str)
        .map(Path::new)
        .ok_or_else(|| anyhow::anyhow!("path is required"))?;
    let path = config.ensure_project_file(path)?;
    let edits = request
        .get("edits")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("edits must be an object"))?;
    if edits.is_empty() {
        bail!("edits must not be empty");
    }
    let original = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let text_bytes = original
        .strip_prefix(&[0xEF, 0xBB, 0xBF])
        .unwrap_or(original.as_slice());
    let text = std::str::from_utf8(text_bytes)
        .with_context(|| format!("file is not UTF-8: {}", path.display()))?;
    let normalized = text.replace("\r\n", "\n");
    let lines = normalized
        .strip_suffix('\n')
        .unwrap_or(&normalized)
        .split('\n')
        .map(str::to_string)
        .collect::<Vec<_>>();
    validate_expected_lines(request, edits, &lines)?;
    let parsed = parse_edits(edits, lines.len())?;
    reject_overlaps(&parsed)
}

fn validate_expected_lines(
    request: &Value,
    edits: &Map<String, Value>,
    lines: &[String],
) -> Result<()> {
    let expected = request
        .get("expected")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "expected must contain the exact current text for every edit range; re-read the file before editing"
            )
        })?;
    if expected.len() != edits.len() || edits.keys().any(|key| !expected.contains_key(key)) {
        bail!("expected must contain exactly the same range keys as edits");
    }
    for (key, expected_value) in expected {
        let expected_text = expected_value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("expected value must be a string: {key}"))?
            .replace("\r\n", "\n");
        let (start, end) = parse_range(key)?;
        if start == 0 || end < start || end > lines.len() {
            bail!(
                "edit range out of bounds: {key} (file has {} lines)",
                lines.len()
            );
        }
        let actual = lines[(start - 1)..end].join("\n");
        if actual != expected_text {
            bail!(
                "edit precondition failed for range {key}; the file changed after it was inspected"
            );
        }
    }
    Ok(())
}

fn parse_edits(edits: &Map<String, Value>, line_count: usize) -> Result<Vec<Edit>> {
    let mut parsed = Vec::new();
    for (key, value) in edits {
        let replacement = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("edit value must be a string: {key}"))?;
        let (start, end) = parse_range(key)?;
        if start == 0 || end < start || end > line_count {
            bail!("edit range out of bounds: {key} (file has {line_count} lines)");
        }
        parsed.push(Edit {
            start,
            end,
            replacement: replacement.to_string(),
        });
    }
    Ok(parsed)
}

fn parse_range(key: &str) -> Result<(usize, usize)> {
    let parts = key.split('-').collect::<Vec<_>>();
    match parts.as_slice() {
        [line] => {
            let line = line
                .trim()
                .parse::<usize>()
                .with_context(|| format!("invalid edit key: {key}"))?;
            Ok((line, line))
        }
        [start, end] => Ok((
            start
                .trim()
                .parse::<usize>()
                .with_context(|| format!("invalid edit key: {key}"))?,
            end.trim()
                .parse::<usize>()
                .with_context(|| format!("invalid edit key: {key}"))?,
        )),
        _ => bail!("invalid edit key: {key}"),
    }
}

fn reject_overlaps(edits: &[Edit]) -> Result<()> {
    // 重叠范围的替换顺序没有唯一答案，直接拒绝而不是猜测用户意图。
    let mut ranges = edits
        .iter()
        .map(|edit| (edit.start, edit.end))
        .collect::<Vec<_>>();
    ranges.sort();
    for pair in ranges.windows(2) {
        if pair[1].0 <= pair[0].1 {
            bail!(
                "overlapping edit ranges: {}-{} and {}-{}",
                pair[0].0,
                pair[0].1,
                pair[1].0,
                pair[1].1
            );
        }
    }
    Ok(())
}
