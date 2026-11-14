//! `picker` 的测试：按 `/` 分层、只展开当前分支所在的目录、最近使用、同一个分支只列一次、搜索与
//! 新建、远程收起。

use super::*;

fn branch(name: &str, remote: bool, current: bool) -> Branch {
    Branch {
        name: name.to_owned(),
        remote,
        current,
        upstream: None,
        gone: false,
        oid: String::new(),
        time: 0,
        local: String::new(),
        ahead: 0,
        behind: 0,
    }
}

fn sample() -> Vec<Branch> {
    vec![
        branch("feature/login", false, true),
        branch("feature/search", false, false),
        branch("fix/deep/one", false, false),
        branch("main", false, false),
        branch("origin/main", true, false),
        branch("origin/feature/login", true, false),
        branch("origin/release/1", true, false),
    ]
}

/// 本地分支一屏放不下的仓库：`main` 和 `b00` 到 `b19`，当前分支是 `b00`。
fn many() -> Vec<Branch> {
    let names = (0..20).map(|i| format!("b{i:02}"));
    let mut branches: Vec<Branch> = names
        .enumerate()
        .map(|(i, name)| branch(&name, false, i == 0))
        .collect();
    branches.push(branch("main", false, false));
    branches
}

/// 排出各行；`query` 同时用作搜索和新建的名字。
fn rows(
    branches: &[Branch],
    recent: &[&str],
    query: &str,
    toggled: &HashSet<(bool, String)>,
) -> Vec<BranchRow> {
    let recent: Vec<String> = recent.iter().map(|&name| name.to_owned()).collect();
    branch_rows(&BranchList {
        branches,
        recent: &recent,
        query: &needle(query),
        text: query,
        toggled,
        tags: &[],
    })
}

/// 把各行写成缩进的文字，便于比较。
fn outline(rows: &[BranchRow]) -> Vec<String> {
    rows.iter()
        .map(|row| match row {
            BranchRow::All => "All".to_owned(),
            BranchRow::Locals { recent: true, .. } => "Recent".to_owned(),
            BranchRow::Locals { recent: false, .. } => "Local".to_owned(),
            BranchRow::Remotes { count, open } => {
                let mark = if *open { "v" } else { ">" };
                format!("{mark} remotes {count}")
            }
            BranchRow::Folder {
                label,
                depth,
                count,
                open,
                ..
            } => {
                let mark = if *open { "v" } else { ">" };
                format!("{}{mark} {label} {count}", "  ".repeat(*depth))
            }
            BranchRow::Tags { count, open } => {
                let mark = if *open { "v" } else { ">" };
                format!("{mark} tags {count}")
            }
            BranchRow::Tag(index) => format!("tag {index}"),
            BranchRow::Branch { depth, label, .. } => format!("{}{label}", "  ".repeat(*depth)),
            BranchRow::Create(name) => format!("+ {name}"),
        })
        .collect()
}

#[test]
fn only_the_current_branchs_folder_starts_open() {
    let rows = rows(&sample(), &[], "", &HashSet::new());
    assert_eq!(
        outline(&rows),
        [
            "Local",
            "v feature 2",
            "  login",
            "  search",
            "> fix/deep 1",
            "main",
            "> remotes 3",
        ]
    );
}

#[test]
fn local_branches_are_listed_by_their_last_commit() {
    let at = |name: &str, time: i64| Branch {
        time,
        ..branch(name, false, false)
    };
    let branches = [
        at("old", 1),
        at("feature/a", 2),
        at("feature/b", 9),
        at("new", 5),
        at("tie-b", 3),
        at("tie-a", 3),
        Branch {
            time: 1,
            ..branch("current", false, true)
        },
        branch("origin/z", true, false),
        branch("origin/a", true, false),
    ];
    let opened = HashSet::from([(true, String::new())]);
    assert_eq!(
        outline(&rows(&branches, &[], "", &opened)),
        [
            "Local",
            "current",
            "> feature 2",
            "new",
            "tie-a",
            "tie-b",
            "old",
            "v remotes 2",
            "a",
            "z",
        ]
    );
}

#[test]
fn toggled_folders_flip_their_default_state() {
    let toggled = HashSet::from([
        (false, "feature".to_owned()),
        (false, "fix/deep".to_owned()),
        (true, String::new()),
        (true, "release".to_owned()),
    ]);
    let rows = rows(&sample(), &[], "", &toggled);
    // 只有一个远程时不写它的名字：少一层没有信息量的目录。已有本地分支的远程分支照样列出。
    assert_eq!(
        outline(&rows),
        [
            "Local",
            "> feature 2",
            "v fix/deep 1",
            "  one",
            "main",
            "v remotes 3",
            "> feature 1",
            "v release 1",
            "  1",
            "main",
        ]
    );
}

#[test]
fn several_remotes_keep_their_names() {
    let mut branches = sample();
    branches.push(branch("fork/topic", true, false));
    let toggled = HashSet::from([(true, String::new())]);
    let rows = rows(&branches, &[], "", &toggled);
    let outline = outline(&rows);
    assert!(outline.contains(&"> origin 3".to_owned()), "{outline:?}");
    assert!(outline.contains(&"> fork 1".to_owned()), "{outline:?}");
}

#[test]
fn few_local_branches_do_not_list_recent() {
    let rows = rows(&sample(), &["main", "feature/search"], "", &HashSet::new());
    assert!(matches!(rows[0], BranchRow::Locals { recent: false, .. }));
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r, BranchRow::Locals { recent: true, .. }))
    );
}

#[test]
fn recent_puts_the_default_branch_first_and_skips_the_current_one() {
    let recent = ["b00", "gone", "b05", "origin/main"];
    let rows = rows(&many(), &recent, "", &HashSet::new());
    assert_eq!(outline(&rows)[..4], ["Recent", "main", "b05", "Local"]);
}

/// 日志里按分支筛选时没有「最近」一组：每个分支只列一次，其余各行不变。
#[test]
fn log_lists_drop_the_recent_group() {
    let all = rows(&many(), &["b05"], "", &HashSet::new());
    let kept = without_recent(all.clone());
    assert_eq!(outline(&kept)[0], "Local");
    let local = all
        .iter()
        .position(|r| matches!(r, BranchRow::Locals { recent: false, .. }));
    let recent = local.unwrap();
    assert_eq!(kept, all[recent..]);
    for name in ["main", "b05"] {
        let times = outline(&kept).iter().filter(|line| *line == name).count();
        assert_eq!(times, 1, "{name}");
    }
}

#[test]
fn recent_branches_stay_in_the_local_group() {
    let outline = outline(&rows(&many(), &["b05"], "", &HashSet::new()));
    for name in ["main", "b05"] {
        let times = outline.iter().filter(|line| *line == name).count();
        assert_eq!(times, 2, "{name}: {outline:?}");
    }
    // 当前分支不在最近使用里，只在下面。
    assert!(outline.contains(&"b00".to_owned()));
}

#[test]
fn remote_branches_are_listed_even_with_a_local_branch() {
    let mut branches = sample();
    // 本地分支跟踪它，名字不一样。
    branches.push(Branch {
        upstream: Some("origin/topic".to_owned()),
        ..branch("work", false, false)
    });
    branches.push(branch("origin/topic", true, false));

    // 远程一组列全：已有同名本地分支的 `main`、被本地分支跟踪的 `topic` 都在。
    let opened = HashSet::from([(true, String::new()), (true, "release".to_owned())]);
    let listed = outline(&rows(&branches, &[], "", &opened));
    let start = listed.iter().position(|l| l == "v remotes 4").unwrap();
    assert_eq!(
        listed[start + 1..],
        ["> feature 1", "v release 1", "  1", "main", "topic"]
    );
}

#[test]
fn recent_keeps_at_most_five_branches() {
    let branches = many();
    let names: Vec<String> = branches.iter().map(|b| b.name.clone()).collect();
    let recent: Vec<&str> = names.iter().map(String::as_str).collect();
    let rows = rows(&branches, &recent, "", &HashSet::new());
    let end = rows
        .iter()
        .position(|r| matches!(r, BranchRow::Locals { recent: false, .. }))
        .unwrap();
    assert_eq!(end - 1, RECENT_BRANCHES);
}

#[test]
fn searching_opens_folders_and_offers_to_create_the_branch() {
    let toggled = HashSet::from([(false, "feature".to_owned())]);
    let found = rows(&sample(), &["main"], "login", &toggled);
    assert_eq!(
        outline(&found),
        [
            "Local",
            "v feature 1",
            "  login",
            "v remotes 1",
            "v feature 1",
            "  login",
            "+ login",
        ]
    );
    // 名字里的空格换成连字符：分支名不能有空格。
    let none = rows(&sample(), &[], "new thing", &HashSet::new());
    assert_eq!(outline(&none), ["+ new-thing"]);
    // 已经有这个分支时不提供新建。
    let exact = rows(&sample(), &[], "main", &HashSet::new());
    assert!(!exact.iter().any(|row| matches!(row, BranchRow::Create(_))));
}

#[test]
fn contiguous_matches_hide_scattered_ones() {
    let branches = [
        branch("feature/login", false, false),
        branch("release/1", false, false),
    ];
    let hit = rows(&branches, &[], "rel", &HashSet::new());
    assert_eq!(outline(&hit), ["Local", "v release 1", "  1", "+ rel"]);
    // 没有连续命中时仍然列出分散命中的。
    let scattered = rows(&branches, &[], "flg", &HashSet::new());
    assert_eq!(
        outline(&scattered),
        ["Local", "v feature 1", "  login", "+ flg"]
    );
}

/// 标签一组排在分支之后，平时收起；展开后按给定的顺序列出，搜索时只列命中的标签。
#[test]
fn tags_follow_the_branches_and_open_on_demand_or_while_searching() {
    let branches = sample();
    let tags = ["v2.0".to_owned(), "v1.1".to_owned(), "beta".to_owned()];
    let list = |query: &str, toggled: &HashSet<(bool, String)>| {
        let recent: Vec<String> = Vec::new();
        let rows = branch_rows(&BranchList {
            branches: &branches,
            recent: &recent,
            query: &needle(query),
            text: query,
            toggled,
            tags: &tags,
        });
        outline(&rows)
    };
    let closed = list("", &HashSet::new());
    assert_eq!(closed.last().map(String::as_str), Some("> tags 3"));
    assert_eq!(closed[closed.len() - 2], "> remotes 3");

    let toggled = HashSet::from([(false, String::new())]);
    let open = list("", &toggled);
    assert_eq!(
        open[open.len() - 4..],
        ["v tags 3", "tag 0", "tag 1", "tag 2"]
    );

    // 搜索时只列命中的标签，并且可以用查询的文字新建分支。
    let found = list("v1", &HashSet::new());
    assert_eq!(found, ["v tags 1", "tag 1", "+ v1"]);
}
