//! Office Open XML 包（xlsx、docx）的公共部分：按路径解压其中的文件、解析 XML、读关系文件。
//!
//! 包是一个 zip 压缩包，各文件之间的引用写在 `_rels` 目录下的关系文件里；格式见 ECMA-376
//! 第 2 部分（Open Packaging Conventions）。

use std::collections::HashMap;

use roxmltree::Document;
use winc_i18n::t;

use crate::zip::Archive;

/// 解压包中的 `path`；没有该文件时返回 `None`。
pub(crate) fn part(archive: &Archive<'_>, path: &str) -> Result<Option<Vec<u8>>, String> {
    archive
        .read(path)
        .transpose()
        .map_err(|_| t!("preview-office-damaged", part = path.to_owned()))
}

/// 把 `bytes` 解析成 XML；`path` 用于报错，也是包中的路径。
pub(crate) fn parse<'a>(bytes: &'a [u8], path: &str) -> Result<Document<'a>, String> {
    let damaged = || t!("preview-office-damaged", part = path.to_owned());
    let text = std::str::from_utf8(bytes).map_err(|_| damaged())?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    Document::parse(text).map_err(|_| damaged())
}

/// 包关系（`_rels/.rels`）中主文档（工作簿、文字文档）在包中的路径。
pub(crate) fn package_target(archive: &Archive<'_>) -> Option<String> {
    let bytes = archive.read("_rels/.rels")?.ok()?;
    let doc = parse(&bytes, "_rels/.rels").ok()?;
    let node = doc.descendants().find(|n| {
        n.tag_name().name() == "Relationship"
            && n.attribute("Type")
                .is_some_and(|t| t.ends_with("/officeDocument"))
    })?;
    Some(resolve("", node.attribute("Target")?))
}

/// `source` 的关系文件（同目录下 `_rels/<文件名>.rels`）中的各条关系。
///
/// 每项是关系编号到（关系类型，目标）的映射：包内的目标是包中的路径，外部目标（超链接）是
/// 原样的地址；没有关系文件时为空。
pub(crate) fn relations(
    archive: &Archive<'_>,
    source: &str,
) -> Result<HashMap<String, (String, String)>, String> {
    let (dir, file) = source.rsplit_once('/').unwrap_or(("", source));
    let path = if dir.is_empty() {
        format!("_rels/{file}.rels")
    } else {
        format!("{dir}/_rels/{file}.rels")
    };
    let Some(bytes) = part(archive, &path)? else {
        return Ok(HashMap::new());
    };
    let doc = parse(&bytes, &path)?;
    Ok(doc
        .descendants()
        .filter(|n| n.tag_name().name() == "Relationship")
        .filter_map(|n| {
            let id = n.attribute("Id")?.to_owned();
            let kind = n.attribute("Type").unwrap_or_default().to_owned();
            let target = n.attribute("Target")?;
            // 外部目标（超链接的网址）不在包内，原样保留。
            let target = if n.attribute("TargetMode") == Some("External") {
                target.to_owned()
            } else {
                resolve(dir, target)
            };
            Some((id, (kind, target)))
        })
        .collect())
}

/// 关系目标 `target` 在包中的路径：以 `/` 开头的从包的根起算，否则相对目录 `dir`。
pub(crate) fn resolve(dir: &str, target: &str) -> String {
    let mut parts: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        dir.split('/').filter(|p| !p.is_empty()).collect()
    };
    for piece in target.split('/') {
        match piece {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(piece),
        }
    }
    parts.join("/")
}
