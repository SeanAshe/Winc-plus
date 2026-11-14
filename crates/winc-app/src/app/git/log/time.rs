//! 提交时间的显示：相对时间、本地时间。

use winc_i18n::t;

use super::*;

/// 本机时区的「2026-10-26 14:32」换成从 1970 年起的秒数（当作 UTC 算，只用来比较、相减）。
pub(super) fn naive_seconds(local: &str) -> Option<i64> {
    let (date, time) = local.split_once(' ')?;
    let mut d = date.split('-').map(str::parse::<i64>);
    let (y, m, day) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    let (h, min) = time.split_once(':')?;
    Some(
        days_from_civil(y, m, day) * 86_400
            + h.parse::<i64>().ok()? * 3600
            + min.parse::<i64>().ok()? * 60,
    )
}

/// 公历日期是 1970-01-01 之后第几天（Howard Hinnant 的算法）。
pub(super) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 日志里显示的时间：一小时内「N 分钟前」，今天「今天 14:32」，昨天「昨天 14:32」，今年
/// 「10月20日」，更早「2024年3月1日」。
///
/// `commit` 是这个提交，`now` 是现在（Unix 秒）。本机时区和
/// UTC 的差从这个提交自己的本地时间算出来（`local` 减 `time`），不用读时区数据库。
pub(crate) fn display_time(commit: &Commit, now: i64) -> String {
    display_local(&commit.local, commit.time, now)
}

/// 日志列表里的时间：今天、昨天的写「今天 10:52」「昨天 10:52」，今年更早的「10-22 10:52」，
/// 更早的「2024-03-01」，用等宽字体画、靠右对齐，一列竖下来时、分一位对一位。
///
/// 月、日补零、不加汉字，列宽才能对齐。几分钟前这类更细的说法只在详情的 [`display_time`]
/// 里写。
pub(crate) fn list_time(commit: &Commit, now: i64) -> String {
    let local = commit.local.as_str();
    let (Some(date), Some(clock)) = (local.get(..10), local.get(11..16)) else {
        return local.to_owned();
    };
    // 「今天」「今年」按这个提交自己的时区算（和 `display_local` 一样，不读时区数据库）。
    let Some(seconds) = naive_seconds(local) else {
        return local.to_owned();
    };
    let offset = seconds - commit.time;
    let (day, today) = (
        seconds.div_euclid(86_400),
        (now + offset).div_euclid(86_400),
    );
    if day == today {
        return t!("git-time-today", time = clock);
    }
    if day + 1 == today {
        return t!("git-time-yesterday", time = clock);
    }
    let (this_year, ..) = civil_from_days(today);
    match date.get(..4).and_then(|y| y.parse::<i64>().ok()) {
        Some(year) if year == this_year => format!("{} {clock}", &date[5..]),
        _ => date.to_owned(),
    }
}

/// 把本地时间的写法「2026-10-26 14:32」换成显示用的文字（相对时间或日期）。
///
/// `time` 是这个提交的 Unix 秒；写法认不出来时原样返回。
pub(crate) fn display_local(local_text: &str, time: i64, now: i64) -> String {
    match naive_seconds(local_text) {
        Some(local) => display_at(local, time, now),
        None => local_text.to_owned(),
    }
}

/// 把本地时间（当作 UTC 算的秒数，见 [`naive_seconds`]）换成显示用的文字。
///
/// `time` 是这个提交的 Unix 秒，`now` 是现在。
pub(crate) fn display_at(local: i64, time: i64, now: i64) -> String {
    let ago = now - time;
    if (0..60).contains(&ago) {
        return t!("git-time-just-now").to_owned();
    }
    if (60..3600).contains(&ago) {
        return t!("git-time-minutes-ago", count = ago / 60);
    }
    let (day, secs) = (local.div_euclid(86_400), local.rem_euclid(86_400));
    let today = (now + local - time).div_euclid(86_400);
    let (y, m, d) = civil_from_days(day);
    let clock = format!("{:02}:{:02}", secs / 3600, secs % 3600 / 60);
    if day == today {
        t!("git-time-today", time = clock)
    } else if day + 1 == today {
        t!("git-time-yesterday", time = clock)
    } else if civil_from_days(today).0 == y {
        t!("git-time-date", month = m, day = d)
    } else {
        t!("git-time-date-year", year = y, month = m, day = d)
    }
}

/// [`display_time`] 是否为相对的说法（刚刚、N 分钟前、今天、昨天），而不只是日期。
pub(crate) fn is_relative_time(commit: &Commit, now: i64) -> bool {
    let Some(local) = naive_seconds(&commit.local) else {
        return false;
    };
    let ago = now - commit.time;
    let day = local.div_euclid(86_400);
    let today = (now + local - commit.time).div_euclid(86_400);
    (0..3600).contains(&ago) || day == today || day + 1 == today
}

/// Unix 秒 `time` 在和 UTC 差 `offset` 秒的时区里的写法「2026-10-26 14:32」。
pub(crate) fn local_text(time: i64, offset: i64) -> String {
    let local = time + offset;
    let (days, secs) = (local.div_euclid(86_400), local.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        secs / 3600,
        secs % 3600 / 60
    )
}

/// 1970-01-01 之后第 `days` 天的（年，月，日）（Howard Hinnant 的算法）。
pub(super) fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
