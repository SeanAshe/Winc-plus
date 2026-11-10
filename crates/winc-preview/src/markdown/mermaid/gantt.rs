//! 甘特图（`gantt`）的解析：分组、任务的起止时间与状态。
//!
//! 时间以 UTC 的秒数表示，不考虑时区。`excludes`、`todayMarker` 等设置被忽略。

use std::collections::HashMap;

use super::label;

/// 甘特图的一项任务。
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    /// 任务名。
    pub name: String,
    /// 所在分组在 [`Gantt::sections`] 中的序号；写在所有 `section` 之前的任务为 `None`。
    pub section: Option<usize>,
    /// 开始时间（秒）。
    pub start: i64,
    /// 结束时间（秒），不早于开始时间。
    pub end: i64,
    /// `done`：已完成。
    pub done: bool,
    /// `active`：进行中。
    pub active: bool,
    /// `crit`：关键任务。
    pub crit: bool,
    /// `milestone`：里程碑，画在起止时间的中点。
    pub milestone: bool,
}

/// 甘特图。
#[derive(Debug, Clone, PartialEq)]
pub struct Gantt {
    /// 图的标题。
    pub title: Option<String>,
    /// 分组名，按源码中的先后排列。
    pub sections: Vec<String>,
    /// 任务，按源码中的先后排列。
    pub tasks: Vec<Task>,
    /// 时间轴刻度的格式（`axisFormat`，`%Y-%m-%d` 这类写法）；没有指定时为 `None`。
    pub axis: Option<String>,
}

/// 一天的秒数。
const DAY: i64 = 86_400;

/// 解析甘特图的正文；没有任何能确定时间的任务时返回 `None`。
pub(super) fn parse(lines: &[&str]) -> Option<Gantt> {
    let mut gantt = Gantt {
        title: None,
        sections: Vec::new(),
        tasks: Vec::new(),
        axis: None,
    };
    let mut format = "YYYY-MM-DD".to_owned();
    // 任务名（`id`）到 [`Gantt::tasks`] 中序号的对应关系。
    let mut ids: HashMap<String, usize> = HashMap::new();
    for &line in lines {
        let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        match word {
            "title" => gantt.title = Some(label(rest)),
            "dateFormat" => format = rest.to_owned(),
            "axisFormat" => gantt.axis = Some(rest.to_owned()),
            "section" => gantt.sections.push(label(rest)),
            "excludes" | "includes" | "todayMarker" | "tickInterval" | "weekday"
            | "inclusiveEndDates" | "topAxis" | "displayMode" | "accTitle" | "accDescr"
            | "click" => {}
            _ => {
                let Some((name, spec)) = line.split_once(':') else {
                    continue;
                };
                let previous = gantt.tasks.last().map(|t| t.end);
                let Some((id, task)) = task(name, spec, &format, previous, &gantt.tasks, &ids)
                else {
                    continue;
                };
                let task = Task {
                    section: gantt.sections.len().checked_sub(1),
                    ..task
                };
                if let Some(id) = id {
                    ids.insert(id.to_owned(), gantt.tasks.len());
                }
                gantt.tasks.push(task);
            }
        }
    }
    (!gantt.tasks.is_empty()).then_some(gantt)
}

/// 解析一行任务 `名字 : 状态, id, 开始, 结束或时长`；返回任务的 `id` 和任务（分组待定）。
///
/// `previous` 是上一项任务的结束时间，没有写开始时间的任务从这里开始。
fn task<'s>(
    name: &str,
    spec: &'s str,
    format: &str,
    previous: Option<i64>,
    tasks: &[Task],
    ids: &HashMap<String, usize>,
) -> Option<(Option<&'s str>, Task)> {
    let mut parts: Vec<&str> = spec.split(',').map(str::trim).collect();
    let mut t = Task {
        name: label(name),
        section: None,
        start: 0,
        end: 0,
        done: false,
        active: false,
        crit: false,
        milestone: false,
    };
    while let Some(&first) = parts.first() {
        match first {
            "done" => t.done = true,
            "active" => t.active = true,
            "crit" => t.crit = true,
            "milestone" => t.milestone = true,
            _ => break,
        }
        parts.remove(0);
    }
    let (id, start, end) = match parts[..] {
        [end] => (None, None, end),
        [start, end] => (None, Some(start), end),
        [id, start, end, ..] => (Some(id), Some(start), end),
        [] => return None,
    };
    let after = |list: &str| {
        list.split_whitespace()
            .filter_map(|id| ids.get(id).map(|&i| tasks[i].end))
            .max()
    };
    t.start = match start {
        Some(s) => match s.strip_prefix("after ") {
            Some(list) => after(list).or(previous)?,
            None => date(s, format)?,
        },
        None => previous?,
    };
    t.end = match end.strip_prefix("until ") {
        Some(list) => list
            .split_whitespace()
            .filter_map(|id| ids.get(id).map(|&i| tasks[i].start))
            .min()?,
        None => match duration(end) {
            Some(d) => t.start + d,
            None => date(end, format)?,
        },
    };
    t.end = t.end.max(t.start);
    Some((id, t))
}

/// 按 `dateFormat`（`YYYY-MM-DD HH:mm` 这类写法）解析时间，返回秒数。
///
/// 支持 `YYYY`、`YY`、`MM`、`M`、`DD`、`D`、`HH`、`H`、`mm`、`m`、`ss`、`s`，以及 Unix 时间
/// `X`（秒）、`x`（毫秒）；其余字符按原样匹配。
fn date(text: &str, format: &str) -> Option<i64> {
    let text = text.trim();
    let (mut year, mut month, mut day, mut hour, mut minute, mut second) = (1970, 1, 1, 0, 0, 0);
    let mut rest = text;
    let mut f = format;
    // 读取至多 `max` 位数字。
    let number = |rest: &mut &str, max: usize| -> Option<i64> {
        let len = rest
            .bytes()
            .take(max)
            .take_while(u8::is_ascii_digit)
            .count();
        let n = rest[..len].parse().ok()?;
        *rest = &rest[len..];
        Some(n)
    };
    while !f.is_empty() {
        let token = [
            "YYYY", "YY", "MM", "M", "DD", "D", "HH", "H", "mm", "m", "ss", "s", "X", "x",
        ]
        .into_iter()
        .find(|t| f.starts_with(t));
        let Some(token) = token else {
            let c = f.chars().next()?;
            rest = rest.strip_prefix(c)?;
            f = &f[c.len_utf8()..];
            continue;
        };
        f = &f[token.len()..];
        match token {
            "YYYY" => year = number(&mut rest, 4)?,
            "YY" => year = 2000 + number(&mut rest, 2)?,
            "MM" | "M" => month = number(&mut rest, 2)?,
            "DD" | "D" => day = number(&mut rest, 2)?,
            "HH" | "H" => hour = number(&mut rest, 2)?,
            "mm" | "m" => minute = number(&mut rest, 2)?,
            "ss" | "s" => second = number(&mut rest, 2)?,
            "X" => return number(&mut rest, 12),
            _ => return Some(number(&mut rest, 15)? / 1000),
        }
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day) * DAY + hour * 3600 + minute * 60 + second)
}

/// 解析时长 `3d`、`1.5w`、`12h`，返回秒数。
///
/// 单位为 `ms`、`s`、`m`（分钟）、`h`、`d`、`w`、`M`（按 30 天）、`y`（按 365 天）。
fn duration(text: &str) -> Option<i64> {
    let text = text.trim();
    let split = text.find(|c: char| !c.is_ascii_digit() && c != '.')?;
    let (n, unit) = text.split_at(split);
    let n: f64 = n.parse().ok()?;
    let seconds = match unit {
        "ms" => 0.001,
        "s" => 1.0,
        "m" => 60.0,
        "h" => 3600.0,
        "d" => DAY as f64,
        "w" => 7.0 * DAY as f64,
        "M" => 30.0 * DAY as f64,
        "y" => 365.0 * DAY as f64,
        _ => return None,
    };
    Some((n * seconds).round() as i64)
}

/// 公历日期到 1970-01-01 的天数。
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 1970-01-01 起的天数对应的公历年、月、日。
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// 时间 `t`（秒）所在月份的第一天零点，再往后 `months` 个月。
pub fn month_start(t: i64, months: i64) -> i64 {
    let (y, m, _) = civil_from_days(t.div_euclid(DAY));
    let index = y * 12 + (m - 1) + months;
    days_from_civil(index.div_euclid(12), index.rem_euclid(12) + 1, 1) * DAY
}

/// 按 `format`（`%Y-%m-%d` 这类写法）格式化时间 `t`（秒）。
///
/// 支持 `%Y`、`%y`、`%m`、`%d`、`%e`、`%H`、`%M`、`%S`、`%b`、`%a`、`%%`，其余字符按原样输出。
pub fn format_time(t: i64, format: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    let days = t.div_euclid(DAY);
    let secs = t.rem_euclid(DAY);
    let (y, m, d) = civil_from_days(days);
    let mut out = String::new();
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => out += &y.to_string(),
            Some('y') => out += &format!("{:02}", y.rem_euclid(100)),
            Some('m') => out += &format!("{m:02}"),
            Some('d') => out += &format!("{d:02}"),
            Some('e') => out += &d.to_string(),
            Some('H') => out += &format!("{:02}", secs / 3600),
            Some('M') => out += &format!("{:02}", secs / 60 % 60),
            Some('S') => out += &format!("{:02}", secs % 60),
            Some('b') => out += MONTHS[(m - 1) as usize],
            Some('a') => out += WEEKDAYS[days.rem_euclid(7) as usize],
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}
