//! 日志标签数据的测试：重取时保留旧提交、换仓库时清空、同一时刻只有一个请求、批量递增、
//! 详情缓存的上限。

use super::*;

fn commit(oid: &str, parents: &[&str]) -> Commit {
    Commit {
        oid: oid.to_owned(),
        parents: parents.iter().map(|p| (*p).to_owned()).collect(),
        author: String::new(),
        email: String::new(),
        time: 0,
        local: String::new(),
        refs: Vec::new(),
        subject: oid.to_owned(),
    }
}

/// 取一批并交回 `commits`（返回交回结果 `Arrival`，供断言判断）。
fn load(view: &mut LogView, commits: Vec<Commit>) -> Arrival {
    let request = view.begin().expect("应当发出请求");
    view.finish(request.generation, Ok(commits))
}

#[test]
fn a_reload_keeps_the_old_commits_until_the_first_batch_arrives() {
    let mut view = LogView::new();
    let arrival = load(
        &mut view,
        vec![commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])],
    );
    assert_eq!(arrival, Arrival::Applied { first: true });
    assert!(view.complete, "不满一批：取完了");
    view.selected = Some(1);
    view.details = Some(Details {
        oid: "b".into(),
        files: Some(Vec::new()),
        ..Details::default()
    });
    assert_eq!(view.next_skip(), 3);

    // 重取后旧提交仍保留，选中项和它的详情也在，下一批从头取。
    view.reset();
    assert!(view.stale && !view.complete);
    assert_eq!(view.commits.len(), 3);
    assert_eq!(view.selected, Some(1));
    assert!(view.details.as_ref().is_some_and(|d| d.files.is_some()));
    assert_eq!(view.next_skip(), 0);

    // 第一批到达时整体替换，提交图从头计算。
    let arrival = load(&mut view, vec![commit("d", &["c"]), commit("c", &["b"])]);
    assert_eq!(arrival, Arrival::Applied { first: true });
    assert!(!view.stale);
    assert_eq!(view.commits.len(), 2);
    assert_eq!(view.graph.len(), 2);
    assert_eq!(view.selected, None);
    assert_eq!(view.commits[0].oid, "d");
    assert_eq!(view.next_skip(), 2);

    // 没有任何提交时重取，不保留旧内容。
    let mut empty = LogView::new();
    empty.reset();
    assert!(!empty.stale);
}

#[test]
fn another_repository_starts_over_and_drops_the_running_request() {
    let mut view = LogView::new();
    load(&mut view, vec![commit("b", &["a"]), commit("a", &[])]);
    view.selected = Some(0);
    view.details = Some(Details {
        oid: "b".into(),
        ..Details::default()
    });
    view.query.text = "fix".into();
    view.compare = Some(Compare::default());
    view.reset();
    let running = view.begin().unwrap();
    let request = view.compare_request;

    view.restart(Some("/other".into()), vec!["main".into()]);
    assert!(view.commits.is_empty() && !view.stale);
    assert!(view.selected.is_none() && view.details.is_none() && view.compare.is_none());
    assert_eq!(view.root.as_deref(), Some(std::path::Path::new("/other")));
    assert_eq!(view.query.branches, ["main"]);
    assert!(view.query.text.is_empty());
    assert_ne!(view.compare_request, request);

    // 旧仓库的那一批交回来时作废，按新仓库再取一次。
    assert_eq!(view.begin(), None);
    let late = vec![commit("old", &[])];
    assert_eq!(view.finish(running.generation, Ok(late)), Arrival::Refetch);
    assert!(view.commits.is_empty());
    assert!(view.begin().is_some());
}

#[test]
fn only_one_request_runs_and_a_reload_meanwhile_fetches_again() {
    let mut view = LogView::new();
    let first = view.begin().unwrap();
    assert_eq!((first.skip, first.max), (0, BATCH));
    assert!(view.is_loading());
    // 已经在取：不再发。
    assert_eq!(view.begin(), None);

    // 取的过程中修改筛选条件：先不发请求，正在运行的那一批结果作废，按新条件再取一次。
    view.reset();
    assert!(!view.is_loading());
    assert_eq!(view.begin(), None);
    assert!(view.is_loading(), "排着队等再取");
    let late = vec![commit("x", &[])];
    assert_eq!(view.finish(first.generation, Ok(late)), Arrival::Refetch);
    assert!(view.commits.is_empty());
    let again = view.begin().unwrap();
    assert_ne!(again.generation, first.generation);

    // 取的过程中重取、之后没有再次请求：交回来的结果丢弃。
    view.reset();
    assert_eq!(
        view.finish(again.generation, Ok(Vec::new())),
        Arrival::Dropped
    );
    assert!(!view.is_loading());

    // 失败时记下原因，不再往下取。
    let request = view.begin().unwrap();
    let arrival = view.finish(request.generation, Err("还没有提交".into()));
    assert_eq!(arrival, Arrival::Applied { first: true });
    assert_eq!(view.error.as_deref(), Some("还没有提交"));
    assert!(view.complete);
    assert_eq!(view.begin(), None);
}

#[test]
fn batches_grow_with_what_was_fetched() {
    let mut view = LogView::new();
    let mut fetched = 0;
    for expected in [BATCH, BATCH, 2 * BATCH, 4 * BATCH] {
        let request = view.begin().unwrap();
        assert_eq!((request.skip, request.max), (fetched, expected));
        let commits = (0..request.max)
            .map(|i| commit(&(fetched + i).to_string(), &[]))
            .collect();
        view.finish(request.generation, Ok(commits));
        assert!(!view.complete, "满了一批：还有");
        fetched += request.max;
    }
    assert_eq!(view.commits.len(), 8 * BATCH);
    // 很深的地方也不超过上限；不满所要的个数时视为取完。
    view.commits
        .extend((0..MAX_BATCH).map(|i| commit(&format!("deep {i}"), &[])));
    let request = view.begin().unwrap();
    assert_eq!(request.max, MAX_BATCH);
    view.finish(request.generation, Ok(vec![commit("last", &[])]));
    assert!(view.complete);
}

#[test]
fn the_details_cache_keeps_the_latest_few() {
    let mut view = LogView::new();
    for i in 0..DETAILS_CACHE + 3 {
        view.remember(Details {
            oid: i.to_string(),
            ..Details::default()
        });
    }
    assert!(view.cached("0").is_none());
    assert!(view.cached("3").is_some());
    // 同一个提交再记一次时替换旧记录。
    view.remember(Details {
        oid: "3".into(),
        message: Some("新".into()),
        ..Details::default()
    });
    assert_eq!(view.cached("3").unwrap().message.as_deref(), Some("新"));
    assert_eq!(view.cache.len(), DETAILS_CACHE);
}

/// 回归：日志每帧在全部提交里找 `HEAD` 所在的行；取回来时记下，重取之后重新记。
#[test]
fn the_head_row_is_noted_as_commits_arrive() {
    let with = |oid: &str, parents: &[&str], refs: Vec<Ref>| Commit {
        refs,
        ..commit(oid, parents)
    };
    let mut view = LogView::new();
    assert_eq!(view.head_row(), None);
    load(
        &mut view,
        vec![
            with("c", &["b"], vec![Ref::Branch("x".into())]),
            with("b", &["a"], vec![Ref::Head("main".into())]),
            commit("a", &[]),
        ],
    );
    assert_eq!(view.head_row(), Some(1));
    // 重取后第一批到达才替换旧内容，`HEAD` 随之重新记录。
    view.reset();
    assert_eq!(view.head_row(), Some(1));
    load(&mut view, vec![with("d", &["c"], vec![Ref::Detached])]);
    assert_eq!(view.head_row(), Some(0));
}
