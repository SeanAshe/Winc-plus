//! 模糊匹配：子序列匹配和打分，文件多时分成几个线程计算。

use super::*;

/// 给 `candidates`（为 `None` 时是整张表）里的每个文件打分，返回命中的 `(分数, 下标)`，
/// 按下标顺序排列。
///
/// 文件数达到 [`PARALLEL_MIN`] 时分成几段，交给后台线程并行计算。
pub(super) fn score_all(
    files: &Arc<Files>,
    needle: &[char],
    candidates: Option<&[u32]>,
) -> Vec<(u32, u32)> {
    let count = candidates.map_or(files.len(), <[u32]>::len);
    let run =
        |files: &Files, needle: &[char], candidates: Option<&[u32]>, from: usize, to: usize| {
            let mut scratch = Scratch::default();
            let mut out = Vec::new();
            for k in from..to {
                let i = candidates.map_or(k as u32, |c| c[k]);
                if let Some(file) = files.get(i as usize)
                    && let Some(s) = score(needle, file, &mut scratch)
                {
                    out.push((s, i));
                }
            }
            out
        };

    let threads = thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(MAX_THREADS);
    if count < PARALLEL_MIN || threads < 2 {
        return run(files, needle, candidates, 0, count);
    }

    // 后台线程拿不到这里借用的数据：文件表原本就是共享的（`Arc`），查询和候选集则需要各复制一次。
    let chunk = count.div_ceil(threads);
    let (files, needle) = (Arc::clone(files), needle.to_vec());
    let candidates: Option<Arc<[u32]>> = candidates.map(Arc::from);
    background::split(count.div_ceil(chunk), threads, move |part| {
        let from = part * chunk;
        run(
            &files,
            &needle,
            candidates.as_deref(),
            from,
            (from + chunk).min(count),
        )
    })
    .into_iter()
    .flatten()
    .collect()
}

/// 归一化查询词：去掉所有空白，反斜杠换成 `/`，截断到 [`MAX_QUERY_CHARS`] 个字符，逐字符转小写。
///
/// 空白不参与匹配，可以用空格分开路径的几部分；反斜杠是 Windows 上复制的路径里的分隔符。
/// 中文没有大小写，转小写对它是恒等变换，因此不需要特殊处理。
pub(super) fn normalize(query: &str) -> String {
    query
        .chars()
        .filter(|c| !c.is_whitespace())
        .take(MAX_QUERY_CHARS)
        .map(|c| if c == '\\' { '/' } else { fold(c) })
        .collect()
}

/// 把一个字符转成小写，保证一对一。
///
/// `str::to_lowercase` 会把个别字符变成两个（`İ` → `i̇`），字符下标就与原路径对不上；
/// 这里只取第一个，查询和路径使用同一个函数，因此匹配结果一致。
fn fold(c: char) -> char {
    if c.is_ascii() {
        c.to_ascii_lowercase()
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

/// 打分时复用的缓冲，整次搜索只分配一次。
#[derive(Default)]
pub(super) struct Scratch {
    /// 路径逐字符转小写后的字符序列。
    lower: Vec<char>,
    /// 路径的原始字符序列，用于判断由小写到大写的词首。
    original: Vec<char>,
    /// 查询每个字符命中的位置（字符下标）。
    positions: Vec<usize>,
}

/// 是否 `needle` 是 `path` 的子序列（大小写不敏感）。
///
/// 不分配内存；大项目里绝大多数路径在这一步就被淘汰。
fn is_subsequence(needle: &[char], path: &str) -> bool {
    // 查询和路径都是 ASCII（常见情形）时按字节比较，省去 UTF-8 解码和 Unicode 折叠。
    // 路径含非 ASCII 字符时不能这样：有些字符折叠后是 ASCII（`İ` → `i`、`K`（开尔文）→ `k`），
    // 按字节比较会漏掉。
    if path.is_ascii() && needle.iter().all(char::is_ascii) {
        let mut rest = needle.iter().map(|&c| c as u8);
        let mut want = rest.next();

        for &b in path.as_bytes() {
            match want {
                Some(w) if b.to_ascii_lowercase() == w => want = rest.next(),
                Some(_) => {}
                None => return true,
            }
        }

        return want.is_none();
    }

    let mut rest = needle.iter().peekable();

    for c in path.chars() {
        match rest.peek() {
            Some(&&want) if fold(c) == want => {
                rest.next();
            }
            Some(_) => {}
            None => return true,
        }
    }

    rest.peek().is_none()
}

/// 给一条路径打分；`needle` 不是路径的子序列时返回 `None`。
///
/// 分数是「命中质量」各项的累加：命中的字符挨在一起、在词首（`/`、`_`、`-`、`.` 后面）、在大写字母
/// 前都加分；命中落在文件名那一段额外加分；命中占路径的比例越高越好，避免长路径靠数量取胜。
///
/// 所有下标都是字符下标：中文路径里一个字占三个字节，混用字节下标会把文件名那一段算错。
pub(super) fn score(needle: &[char], file: IndexedFile<'_>, scratch: &mut Scratch) -> Option<u32> {
    align(needle, file, scratch, None)
}

/// 返回 `needle` 在路径里命中的字符下标（对齐方式与打分时相同），面板据此标出命中的字符；
/// 不命中时返回 `None`。
///
/// 只对界面上显示的几行计算，不在打分的耗时路径上。
pub(super) fn matched(needle: &[char], file: IndexedFile<'_>) -> Option<Vec<usize>> {
    let mut best = Vec::new();
    align(needle, file, &mut Scratch::default(), Some(&mut best))?;
    Some(best)
}

/// [`score`] 的实现；`keep` 不为 `None` 时把分数最高的那种对齐方式的命中位置存进去。
fn align(
    needle: &[char],
    file: IndexedFile<'_>,
    scratch: &mut Scratch,
    mut keep: Option<&mut Vec<usize>>,
) -> Option<u32> {
    if needle.is_empty() {
        return Some(0);
    }

    // 先做不分配内存的子序列检查，通过的路径才展开成字符表细算。
    if !is_subsequence(needle, file.path) {
        return None;
    }

    let Scratch {
        lower,
        original,
        positions,
    } = scratch;
    original.clear();
    original.extend(file.path.chars());
    lower.clear();
    lower.extend(original.iter().map(|&c| fold(c)));
    positions.clear();

    let haystack = &lower[..];
    let name_start = haystack.len() - file.name.chars().count();

    // 先从前往后找出每个字符第一次出现的位置。
    let mut hay = 0;
    for &c in needle {
        let found = haystack[hay..].iter().position(|&h| h == c)?;
        hay += found;
        positions.push(hay);
        hay += 1;
    }

    // 每个查询字符的命中位置有两种对齐方式，各算一遍分取较高者：往回收（`src/lib.rs` 中找
    // `src` 时命中开头连续的三个字符）与从路径末尾往回找（`button/button.ts` 中找 `button`
    // 时命中文件名内部）。只用其中一种时另一种情形会选错；往回找时每个字符一定找得到。
    let forward = hay;
    let mut best = 0;
    for limit in [forward, haystack.len()] {
        let mut limit = limit;
        for i in (0..positions.len()).rev() {
            let c = needle[i];
            let pos = haystack[..limit].iter().rposition(|&h| h == c)?;
            positions[i] = pos;
            limit = pos;
        }
        let points = rate(positions, haystack, original, name_start, needle.len());
        if let Some(keep) = keep.as_deref_mut()
            && (points > best || keep.is_empty())
        {
            keep.clone_from(positions);
        }
        best = best.max(points);
    }
    Some(best)
}

/// 计算一种对齐方式（每个查询字符命中的位置 `positions`）的分数。
///
/// 各项权重取自调参，没有单位，只表示相对大小，相对顺序由本文件的测试验证。
fn rate(
    positions: &[usize],
    haystack: &[char],
    original: &[char],
    name_start: usize,
    needle: usize,
) -> u32 {
    let mut total: u32 = 0;
    let mut run: u32 = 0;
    let whole_name = positions[0] >= name_start;

    for (i, &pos) in positions.iter().enumerate() {
        let mut points: u32 = 10;

        // 词首：路径开头、分隔符后面，或小写到大写的转换处（`fooBar` 中的 `B`）。
        let boundary = pos == 0
            || matches!(haystack[pos - 1], '/' | '_' | '-' | '.' | ' ')
            || (original[pos].is_uppercase() && original[pos - 1].is_lowercase());
        if boundary {
            points += 15;
        }

        // 连续命中的字符越多，加分越大（逐级递增），以奖励整段命中。
        if i > 0 && positions[i - 1] + 1 == pos {
            run += 1;
            points += 15 * run;
        } else {
            run = 0;
        }

        // 命中在文件名那一段。
        if pos >= name_start {
            points += 10;
        }

        // 查询整个落在文件名内：用户多半按文件名查找。
        if whole_name {
            points += 5;
        }

        total += points;
    }

    // 命中越靠前、占路径的比例越高，分数越高；路径越长，扣分越多。
    let span = positions[positions.len() - 1] - positions[0] + 1;
    let density = (needle as u32 * 20 / span.max(1) as u32).min(20);
    let start_bonus = 20u32.saturating_sub(positions[0] as u32);
    let length_penalty = (haystack.len() as u32 / 8).min(30);

    (total + density + start_bonus).saturating_sub(length_penalty)
}
