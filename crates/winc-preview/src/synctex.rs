//! 读 TeX 写出的 `.synctex.gz`：源码的行与 PDF 上位置（bp，从页面左上角算起）的对照。

use std::path::{Path, PathBuf};

/// 1 个 PostScript 点（bp）是多少 TeX 的 sp。
const SP_PER_BP: f64 = 65781.76;
/// 读入的对照最大的字节数（解压后）。
const MAX_TEXT: usize = 256 << 20;

/// 一条位置记录：源码的哪一行排在哪一页的什么位置（bp，从页面左上角算起）。
#[derive(Debug, Clone, Copy, PartialEq)]
struct Record {
    input: u32,
    line: u32,
    page: u32,
    h: f32,
    v: f32,
    /// 是否是排出的内容而不是盒子。
    content: bool,
}

/// 读出的对照。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Synctex {
    /// 输入文件的编号和路径（按编译时的工作目录解析）。
    inputs: Vec<(u32, PathBuf)>,
    records: Vec<Record>,
}

impl Synctex {
    /// 读 `bytes`（gzip 或文本），相对路径按工作目录 `dir` 解析。
    pub fn read(bytes: &[u8], dir: &Path) -> Option<Synctex> {
        let text = if bytes.starts_with(&[0x1f, 0x8b]) {
            let body = gzip_body(bytes)?;
            miniz_oxide::inflate::decompress_to_vec_with_limit(body, MAX_TEXT).ok()?
        } else {
            bytes.to_vec()
        };
        Some(Synctex::parse(&String::from_utf8_lossy(&text), dir))
    }

    /// 解析文本形式的对照。
    pub fn parse(text: &str, dir: &Path) -> Synctex {
        let mut out = Synctex::default();
        let (mut unit, mut magnification) = (1.0_f64, 1000.0_f64);
        let (mut x_offset, mut y_offset) = (0.0_f64, 0.0_f64);
        let mut page = 0u32;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("Input:") {
                // 空路径不记。
                if let Some((tag, path)) = rest.split_once(':')
                    && let Ok(tag) = tag.parse()
                    && !path.is_empty()
                {
                    let path = Path::new(path.strip_prefix("./").unwrap_or(path));
                    out.inputs.push((tag, dir.join(path)));
                }
                continue;
            }
            let number = |key: &str| {
                line.strip_prefix(key)
                    .and_then(|v| v.trim().parse::<f64>().ok())
            };
            if let Some(v) = number("Unit:") {
                unit = v;
            } else if let Some(v) = number("Magnification:") {
                magnification = v;
            } else if let Some(v) = number("X Offset:") {
                x_offset = v;
            } else if let Some(v) = number("Y Offset:") {
                y_offset = v;
            }
            let Some(kind) = line.chars().next() else {
                continue;
            };
            let body = &line[kind.len_utf8()..];
            match kind {
                '{' => page = body.trim().parse().unwrap_or(page),
                '[' | '(' | 'h' | 'x' | 'k' | 'g' | '$' => {
                    let scale = unit * magnification / 1000.0 / SP_PER_BP;
                    let bp = |v: f64, offset: f64| ((v + offset) * scale) as f32;
                    if let Some((input, number, h, v)) = record(body) {
                        out.records.push(Record {
                            input,
                            line: number,
                            page,
                            h: bp(h, x_offset),
                            v: bp(v, y_offset),
                            content: matches!(kind, 'x' | 'k' | 'g' | '$'),
                        });
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// 返回文件 `file` 第 `line` 行（从 1 起）的位置：（页，从 0 起；横，纵）。
    pub fn forward(&self, file: &Path, line: usize) -> Option<(usize, f32, f32)> {
        let tags: Vec<u32> = (self.inputs.iter())
            .filter(|(_, path)| same_file(path, file))
            .map(|(tag, _)| *tag)
            .collect();
        let line = line as u32;
        // 盒子的行号不可靠，有内容的记录时只看内容。
        let content = (self.records.iter()).any(|r| r.content && tags.contains(&r.input));
        let ours = || {
            (self.records.iter())
                .filter(|r| tags.contains(&r.input) && r.line > 0 && (r.content || !content))
        };
        let before = ours().filter(|r| r.line <= line).map(|r| r.line).max();
        let after = || ours().filter(|r| r.line > line).map(|r| r.line).min();
        let target = before.or_else(after)?;
        let first = ours().filter(|r| r.line == target).min_by(|a, b| {
            (a.page, a.v)
                .partial_cmp(&(b.page, b.v))
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
        Some((first.page.saturating_sub(1) as usize, first.h, first.v))
    }

    /// 返回第 `page` 页（从 0 起）上 (`h`, `v`) 处的文件和行（从 1 起）。
    pub fn inverse(&self, page: usize, h: f32, v: f32) -> Option<(PathBuf, usize)> {
        let page = page as u32 + 1;
        let score = |r: &Record| (r.v - v).abs() * 8.0 + (r.h - h).abs();
        let on_page = || (self.records.iter()).filter(|r| r.page == page && r.line > 0);
        let content = on_page().any(|r| r.content);
        let best = on_page().filter(|r| r.content || !content).min_by(|a, b| {
            score(a)
                .partial_cmp(&score(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
        let path = self
            .inputs
            .iter()
            .find(|(tag, _)| *tag == best.input)?
            .1
            .clone();
        Some((path, best.line as usize))
    }
}

/// `编号,行:横,纵` 开头的记录（后面的宽高、列号不用）。
fn record(body: &str) -> Option<(u32, u32, f64, f64)> {
    let (link, point) = body.split_once(':')?;
    let (tag, line) = link.split_once(',')?;
    // `行,列` 时只取行。
    let line = line.split(',').next()?;
    let point = point.split(':').next()?;
    let (h, v) = point.split_once(',')?;
    Some((
        tag.parse().ok()?,
        line.parse().ok()?,
        h.parse().ok()?,
        v.parse().ok()?,
    ))
}

/// 两个路径是否指同一个文件：去掉 `.`、`..` 后相同。
fn same_file(a: &Path, b: &Path) -> bool {
    normalize(a) == normalize(b)
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// 返回 gzip 文件中压缩数据的部分。
fn gzip_body(bytes: &[u8]) -> Option<&[u8]> {
    let flags = *bytes.get(3)?;
    if bytes.get(2) != Some(&8) {
        return None;
    }
    let mut at = 10;
    if flags & 4 != 0 {
        let len = u16::from_le_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]) as usize;
        at += 2 + len;
    }
    for flag in [8, 16] {
        if flags & flag != 0 {
            at += bytes.get(at..)?.iter().position(|&b| b == 0)? + 1;
        }
    }
    if flags & 2 != 0 {
        at += 2;
    }
    bytes.get(at..)
}

#[cfg(test)]
mod tests;
