//! 快速打开的文件索引的测试：排序与按路径定位、模糊匹配的打分与对齐、并行打分与单线程一致、
//! 按层收录与增量更新遵守忽略规则。

use super::*;

/// 按给定顺序放进文件表（不排序）。
fn index(paths: &[&str]) -> FileIndex {
    let mut files = Files::default();
    for path in paths {
        files.push(path);
    }
    FileIndex {
        root: Some(PathBuf::from("/tmp/winc-test")),
        files: Arc::new(files),
        ..FileIndex::default()
    }
}

fn paths(idx: &FileIndex) -> Vec<&str> {
    (0..idx.len()).map(|i| idx.file(i).unwrap().path).collect()
}

#[test]
fn sorted_case_insensitively_and_found_by_path() {
    let root = crate::test_util::scratch("picker-order");
    std::fs::create_dir_all(root.join("src")).unwrap();
    for name in [
        "b.rs",
        "Cargo.toml",
        "a.txt",
        "src/Z.rs",
        "src/y.rs",
        "README",
    ] {
        std::fs::write(root.join(name), "").unwrap();
    }
    let index = FileIndex::collect_now(&root);
    let paths: Vec<&str> = (0..index.len())
        .map(|i| index.file(i).unwrap().path)
        .collect();
    assert_eq!(
        paths,
        [
            "a.txt",
            "b.rs",
            "Cargo.toml",
            "README",
            "src/y.rs",
            "src/Z.rs"
        ]
    );
    let root = index.root().unwrap().to_path_buf();
    for (i, path) in paths.iter().enumerate() {
        assert_eq!(index.position_of(&root.join(path)), Some(i));
    }
    assert_eq!(index.position_of(&root.join("src/z.rs")), None);
    assert_eq!(index.position_of(Path::new("/elsewhere/a.txt")), None);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn matches_the_file_name_over_the_directory() {
    let mut idx = index(&["src/bcd/other.rs", "src/abc.rs"]);
    let hits = idx.search("abc");
    assert_eq!(hits.len(), 1);
    assert_eq!(idx.file(hits[0]).unwrap().path, "src/abc.rs");
}

/// 面板里标出来的字与打分时选的对齐方式一致：`button` 落进文件名，`src` 落在开头，中文按
/// 字符数计下标，不命中时为空。
#[test]
fn matched_characters_follow_the_chosen_alignment() {
    let idx = index(&["button/button.ts", "src/lib.rs", "文档/说明.md", "a.rs"]);
    assert_eq!(idx.matched("button", 0), (7..13).collect::<Vec<_>>());
    assert_eq!(idx.matched("src", 1), vec![0, 1, 2]);
    assert_eq!(idx.matched("Lib", 1), vec![4, 5, 6]);
    assert_eq!(idx.matched("说明", 2), vec![3, 4]);
    assert!(idx.matched("zzz", 3).is_empty());
    assert!(idx.matched("a", 9).is_empty());
}

/// 命中数是截断之前的数，查询为空时没有。
#[test]
fn matches_counts_every_hit_before_truncating() {
    let names: Vec<String> = (0..MAX_RESULTS + 5).map(|i| format!("f{i}.rs")).collect();
    let mut idx = index(&names.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(idx.search("rs").len(), MAX_RESULTS);
    assert_eq!(idx.matches(), Some(MAX_RESULTS + 5));
    idx.search("f1");
    assert_eq!(idx.matches(), Some(idx.search("f1").len()));
    assert!(idx.search("zzz").is_empty());
    assert_eq!(idx.matches(), Some(0));
    idx.search("");
    assert_eq!(idx.matches(), None);
}

#[test]
fn a_query_that_is_not_a_subsequence_matches_nothing() {
    let mut idx = index(&["src/main.rs", "src/lib.rs"]);
    assert!(idx.search("zzz").is_empty());
    assert!(idx.search("mian").is_empty());
    // 字符顺序颠倒时不算子序列。
    assert!(idx.search("nim").is_empty());
}

#[test]
fn adjacent_characters_beat_scattered_ones() {
    let mut idx = index(&["src/m_a_i_n.rs", "src/main.rs"]);
    let hits = idx.search("main");
    assert_eq!(idx.file(hits[0]).unwrap().path, "src/main.rs");
}

#[test]
fn word_starts_beat_mid_word_hits() {
    let mut idx = index(&["src/xxmxxaxxiyxxn.rs", "src/my_awesome_thing.rs"]);
    let hits = idx.search("mat");
    assert_eq!(idx.file(hits[0]).unwrap().path, "src/my_awesome_thing.rs");
}

#[test]
fn shorter_paths_win_on_equal_scores() {
    let mut idx = index(&["a/very/deep/nested/path/to/file.rs", "file.rs"]);
    let hits = idx.search("file");
    assert_eq!(idx.file(hits[0]).unwrap().path, "file.rs");
}

#[test]
fn query_is_case_insensitive() {
    let mut idx = index(&["src/Main.rs"]);
    assert_eq!(idx.search("main").len(), 1);
    assert_eq!(idx.search("MAIN").len(), 1);
    assert_eq!(idx.search("MaIn").len(), 1);
}

#[test]
fn chinese_file_names_match() {
    let mut idx = index(&["文档/设计稿说明.md", "src/main.rs"]);
    let hits = idx.search("设计");
    assert_eq!(hits.len(), 1);
    assert_eq!(idx.file(hits[0]).unwrap().path, "文档/设计稿说明.md");
}

#[test]
fn empty_query_lists_files_in_index_order() {
    let mut idx = index(&["b.rs", "a.rs"]);
    assert_eq!(idx.search(""), vec![0, 1]);
    assert_eq!(idx.search("   "), vec![0, 1]);
}

#[test]
fn file_name_bonus_counts_characters_not_bytes() {
    // 中文目录名一个字占三个字节。若按字节计算文件名的起点，起点会偏进目录部分，
    // 目录里的 `a` 与文件名里的 `a` 就无法区分。
    let mut idx = index(&["很长的中文目录名/b/a.rs", "很长的中文目录名/a/b.rs"]);
    let hits = idx.search("a");
    assert_eq!(idx.file(hits[0]).unwrap().path, "很长的中文目录名/b/a.rs");
}

#[test]
fn the_file_name_wins_when_the_query_is_also_a_folder_name() {
    // 目录和文件同名（`button/button.ts`）：命中落进文件名的那条排前面。
    let mut idx = index(&["components/button/index.ts", "components/button/button.ts"]);
    let hits = idx.search("button");
    assert_eq!(
        idx.file(hits[0]).unwrap().path,
        "components/button/button.ts"
    );

    // 向文件名一侧靠拢时不能拆散开头连续的命中：`src` 仍排在前面。
    let mut idx = index(&["src/lib/rs.c", "lib/x/s/r/c.rs"]);
    let first = idx.search("src")[0];
    assert_eq!(idx.file(first).unwrap().path, "src/lib/rs.c");
}

#[test]
fn camel_case_humps_are_word_starts() {
    // `PB` 命中 `PickerBar` 的两个大写字母（都是词首），分数高于散落在小写字母中的命中。
    let mut idx = index(&["src/PickerBar.rs", "src/pxxb.rs"]);
    let hits = idx.search("pb");
    assert_eq!(idx.file(hits[0]).unwrap().path, "src/PickerBar.rs");
}

#[test]
fn folding_keeps_one_char_per_char() {
    // `İ` 的标准小写是两个字符，逐字符折叠只取一个，下标不会错位，也不会 panic。
    let mut idx = index(&["İstanbul/notes.md", "src/main.rs"]);
    let hits = idx.search("notes");
    assert_eq!(idx.file(hits[0]).unwrap().path, "İstanbul/notes.md");
    assert_eq!(idx.search("istanbul").len(), 1);

    // 文件名起点不能用「小写路径字节数 − 原文件名字节数」计算：`ẞ` 转小写后字节数变少
    // （3 → 2），减法会得到负数。
    let mut idx = index(&["ẞ.md"]);
    assert_eq!(idx.search("md").len(), 1);
}

#[test]
fn results_are_capped() {
    let paths: Vec<String> = (0..MAX_RESULTS + 50).map(|i| format!("f{i}.rs")).collect();
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let mut idx = index(&refs);
    assert_eq!(idx.search("").len(), MAX_RESULTS);
    assert_eq!(idx.search("f").len(), MAX_RESULTS);
}

/// 构造一张足够大的表（超过 [`PARALLEL_MIN`]，会分到多个线程打分），路径形式多样。
fn big_index() -> FileIndex {
    let words = [
        "src",
        "App",
        "main",
        "lib_utils",
        "V29__backfill",
        "设计稿",
        "test-data",
        "İx",
    ];
    let paths: Vec<String> = (0..PARALLEL_MIN * 2 + 7)
        .map(|i| {
            let a = words[i % words.len()];
            let b = words[(i / 7) % words.len()];
            format!("{a}/{b}{}/{}_{i}.rs", i % 13, words[(i / 3) % words.len()])
        })
        .collect();
    FileIndex {
        root: Some(PathBuf::from("/tmp/winc-test")),
        files: Arc::new(Files::sorted(
            &paths.iter().map(String::as_str).collect(),
            usize::MAX,
        )),
        ..FileIndex::default()
    }
}

#[test]
fn typing_on_narrows_to_the_same_results_as_a_fresh_search() {
    let mut typing = big_index();
    // 逐个字符追加查询，中间删去一个字符，再更换开头。
    for query in [
        "v", "v2", "v29", "v29_", "v29__b", "v29_", "v29_x", "s", "sm", "smai", "设", "设计",
    ] {
        let narrowed = typing.search(query);
        let mut fresh = big_index();
        assert_eq!(narrowed, fresh.search(query), "{query}");
    }
}

#[test]
fn scoring_on_several_threads_matches_one_thread() {
    let idx = big_index();
    for query in ["a", "src", "v29b", "设计", "ix", "zzz"] {
        let needle: Vec<char> = normalize(query).chars().collect();
        let mut scratch = Scratch::default();
        let one: Vec<(u32, u32)> = (0..idx.len())
            .filter_map(|i| {
                let file = idx.files.get(i).unwrap();
                score(&needle, file, &mut scratch).map(|s| (s, i as u32))
            })
            .collect();
        assert_eq!(score_all(&idx.files, &needle, None), one, "{query}");
    }
}

#[test]
fn absolute_paths_join_the_root() {
    let idx = index(&["src/main.rs"]);
    let absolute = idx.absolute(0).unwrap();
    assert!(absolute.ends_with("src/main.rs") || absolute.ends_with("src\\main.rs"));
}

#[test]
fn collect_respects_gitignore_and_hidden_directories() {
    let dir = crate::test_util::scratch("index");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("target")).unwrap();
    std::fs::create_dir_all(dir.join(".git")).unwrap();

    std::fs::write(dir.join("src/main.rs"), "fn main() {}").unwrap();
    std::fs::write(dir.join("README.md"), "# 说明").unwrap();
    std::fs::write(dir.join("target/junk.rs"), "junk").unwrap();
    std::fs::write(dir.join(".git/config"), "[core]").unwrap();
    std::fs::write(dir.join(".gitignore"), "target/\n").unwrap();

    let idx = FileIndex::collect_now(&dir);
    let paths = paths(&idx);

    assert!(paths.contains(&"src/main.rs"), "{paths:?}");
    assert!(paths.contains(&"README.md"), "{paths:?}");
    assert!(!paths.contains(&"target/junk.rs"), "{paths:?}");
    assert!(!paths.contains(&".git/config"), "{paths:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 回归：版本库根目录之外的 `.gitignore` 曾把版本库里的文件也隐藏掉。
#[test]
fn ignore_rules_above_a_repository_do_not_apply_inside_it() {
    let outer = crate::test_util::scratch("index-repo-boundary");
    std::fs::write(outer.join(".gitignore"), "dist/\n").unwrap();
    write(&outer, "repo/.git/HEAD");
    write(&outer, "repo/.gitignore");
    write(&outer, "repo/dist/core/a.d.ts");
    write(&outer, "repo/target/out.o");
    std::fs::write(outer.join("repo/.gitignore"), "target/\n").unwrap();

    let repo = outer.join("repo");
    let index = FileIndex::collect_now(&repo);
    let listed = paths(&index);
    assert!(listed.contains(&"dist/core/a.d.ts"), "{listed:?}");
    // 版本库自己的规则照常生效。
    assert!(!listed.contains(&"target/out.o"), "{listed:?}");
    // 按目录的变化更新文件表时，用的是同一套规则。
    let cancel = AtomicBool::new(false);
    let dist = repo.join("dist");
    assert!(patch(&repo, &index.files, std::slice::from_ref(&dist), &cancel).is_none());

    // 打开版本库里的子目录：版本库根目录的规则仍然生效。
    write(&outer, "repo/sub/target/b.o");
    write(&outer, "repo/sub/c.rs");
    let index = FileIndex::collect_now(&repo.join("sub"));
    assert_eq!(paths(&index), ["c.rs"]);

    let _ = std::fs::remove_dir_all(&outer);
}

/// 没有 `.git` 的目录（如下载的源码包）里，它自己的 `.gitignore` 同样生效。
#[test]
fn a_folder_without_a_repository_keeps_its_own_ignore_rules() {
    let root = crate::test_util::scratch("index-no-repo");
    write(&root, "node_modules/x.js");
    write(&root, "a.rs");
    std::fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
    let index = FileIndex::collect_now(&root);
    assert!(!paths(&index).contains(&"node_modules/x.js"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn queries_ignore_spaces_and_accept_backslashes() {
    let root = crate::test_util::scratch("index-query-forms");
    write(&root, "dist/core/account-store.d.ts");
    let mut index = FileIndex::collect_now(&root);
    for query in [
        "account store",
        "dist core account",
        " account-store ",
        "core\\account-store",
    ] {
        assert_eq!(index.search(query).len(), 1, "{query}");
        assert!(!index.matched(query, 0).is_empty(), "{query}");
    }
    assert!(index.search("account zebra").is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn walking_layer_by_layer_follows_the_same_ignore_rules_as_the_walker() {
    let root = crate::test_util::scratch("index-rules");
    for path in [
        "a.rs",
        "build/out.o",
        "build/keep.txt",
        "logs/x.log",
        "logs/important.log",
        "src/main.rs",
        "src/gen/big.rs",
        "src/gen/small.rs",
        "web/node_modules/pkg/index.js",
        "web/app.ts",
        "web/dist/app.js",
        "notes/.hidden/secret.md",
        "notes/today.md",
        "repo/.git/HEAD",
        "repo/excluded.txt",
        "repo/kept.txt",
        "__pycache__/x.pyc",
        "deep/a/b/c/d/e/f.txt",
    ] {
        write(&root, path);
    }
    // 覆盖的规则：取反、只管目录的规则、子目录自己的规则、`.ignore`、版本库的 exclude。
    std::fs::write(
        root.join(".gitignore"),
        "build/\n!build/keep.txt\n*.log\n!important.log\nnode_modules/\n",
    )
    .unwrap();
    std::fs::write(root.join("src/.gitignore"), "gen/*\n!gen/small.rs\n").unwrap();
    std::fs::write(root.join("web/.ignore"), "dist\n").unwrap();
    std::fs::write(root.join("repo/.git/info_exclude_dummy"), "").unwrap();
    std::fs::create_dir_all(root.join("repo/.git/info")).unwrap();
    std::fs::write(root.join("repo/.git/info/exclude"), "excluded.txt\n").unwrap();

    let cancel = AtomicBool::new(false);
    let table = collect(&root, MAX_FILES, &cancel);
    let mut walked: Vec<String> = walker(&root)
        .filter_entry(|e| {
            e.depth() == 0 || !HIDDEN.contains(&e.file_name().to_string_lossy().as_ref())
        })
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| is_listed_file(t, e.path())))
        .filter_map(|e| e.path().strip_prefix(&root).ok().and_then(relative_path))
        .collect();
    walked.sort_by(|a, b| path_order(a, b));
    assert_eq!(contents(&table).0, walked);
    assert!(
        walked.contains(&"build/keep.txt".to_owned())
            || !walked.contains(&"build/out.o".to_owned())
    );
    assert!(walked.contains(&"src/gen/small.rs".to_owned()));
    assert!(!walked.contains(&"src/gen/big.rs".to_owned()));
    assert!(!walked.contains(&"repo/excluded.txt".to_owned()));
    assert!(walked.contains(&"deep/a/b/c/d/e/f.txt".to_owned()));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn too_many_files_keeps_whole_shallow_layers() {
    let dir = crate::test_util::scratch("index-layers");
    // 第 1 层 2 个，第 2 层 4 个，第 3 层 8 个，第 4 层 16 个。
    for (depth, count) in [(1, 2), (2, 4), (3, 8), (4, 16)] {
        for i in 0..count {
            let folder: PathBuf = (1..depth).map(|d| format!("d{}", (i + d) % 2)).collect();
            std::fs::create_dir_all(dir.join(&folder)).unwrap();
            std::fs::write(dir.join(folder).join(format!("f{depth}_{i}.txt")), "").unwrap();
        }
    }
    let depth = |path: &str| path.matches('/').count() + 1;
    let cancel = AtomicBool::new(false);

    // 放得下全部文件：不分层。
    let all = collect(&dir, 100, &cancel);
    assert_eq!((all.len(), all.layers), (30, None));

    // 前三层（14 个）放得下，四层（30 个）放不下：只收录三层。
    let some = collect(&dir, 20, &cancel);
    assert_eq!((some.len(), some.layers), (14, Some(3)));
    let paths: Vec<&str> = (0..some.len()).map(|i| some.get(i).unwrap().path).collect();
    assert!(paths.iter().all(|p| depth(p) <= 3), "{paths:?}");

    // 遍历过的目录只到第 2 层（第 3 层的文件在它们里面），第 3 层的目录没有遍历过。
    assert_eq!(contents(&some).1, ["d0", "d0/d1", "d1", "d1/d0"]);

    // 重新遍历，结果相同。
    let again = collect(&dir, 20, &cancel);
    assert_eq!(contents(&again), contents(&some));

    // 第 1 层自己就放不下：按路径顺序收录前面的。
    let first = collect(&dir, 1, &cancel);
    assert_eq!((first.len(), first.layers), (1, Some(1)));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn path_order_matches_comparing_lowercase_chars() {
    fn reference(a: &str, b: &str) -> std::cmp::Ordering {
        a.chars()
            .flat_map(char::to_lowercase)
            .cmp(b.chars().flat_map(char::to_lowercase))
            .then_with(|| a.cmp(b))
    }
    // 词尾的 Σ、转小写后变成两个字符的 İ、转小写后是 ASCII 的开尔文符号 K，以及中文。
    let words = [
        "",
        "a",
        "A",
        "a.txt",
        "A.txt",
        "ab",
        "aB",
        "Cargo.toml",
        "cargo.lock",
        "src/a.rs",
        "src/B.rs",
        "src/b.rs",
        "ΣΑΣ",
        "σας",
        "İstanbul",
        "i̇stanbul",
        "istanbul",
        "\u{212A}elvin",
        "kelvin",
        "Kelvin",
        "中文/说明.md",
        "中文/README.md",
        "z",
        "_",
        "-",
        "a/b",
        "a-b",
    ];
    for a in words {
        for b in words {
            assert_eq!(path_order(a, b), reference(a, b), "{a:?} {b:?}");
        }
    }
}

/// 返回表中的文件和遍历过的目录，均已排序。
fn contents(files: &Files) -> (Vec<&str>, Vec<&str>) {
    let dirs: Vec<&str> = files.dirs.iter().collect();
    (
        (0..files.len())
            .map(|i| files.get(i).unwrap().path)
            .collect(),
        dirs,
    )
}

fn write(root: &Path, path: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "x").unwrap();
}

#[test]
fn patching_with_the_changed_paths_matches_walking_again() {
    let root = crate::test_util::scratch("index-patch");
    for path in [
        "src/a.rs",
        "src/deep/b.rs",
        "docs/x.md",
        "target/junk.rs",
        "x",
        "sub/keep.rs",
        "sub/drop.tmp",
    ] {
        write(&root, path);
    }
    std::fs::write(root.join(".gitignore"), "target/\n*.log\n").unwrap();
    let cancel = AtomicBool::new(false);
    let base = collect(&root, MAX_FILES, &cancel);

    // 只修改了内容：文件表不需要变。
    std::fs::write(root.join("src/a.rs"), "changed").unwrap();
    assert!(patch(&root, &base, &[root.join("src/a.rs")], &cancel).is_none());
    // 被忽略的目录里的变化与文件表无关。
    write(&root, "target/new.rs");
    write(&root, "notes.log");
    let ignored = [root.join("target/new.rs"), root.join("notes.log")];
    assert!(patch(&root, &base, &ignored, &cancel).is_none());

    // 依次覆盖：新建、删除、目录改名、新建多层目录、文件换成同名目录、子目录里新增忽略规则。
    write(&root, "src/b.rs");
    std::fs::remove_file(root.join("docs/x.md")).unwrap();
    std::fs::rename(root.join("src"), root.join("source")).unwrap();
    write(&root, "lib/nested/c.rs");
    std::fs::remove_file(root.join("x")).unwrap();
    write(&root, "x/inside.rs");
    std::fs::write(root.join("sub/.gitignore"), "*.tmp\n").unwrap();
    // 通知送来的路径：改名对应新旧两个路径，新建的目录和里面的文件各一个路径。
    let changed: Vec<PathBuf> = [
        "src/b.rs",
        "docs/x.md",
        "src",
        "source",
        "lib",
        "lib/nested",
        "lib/nested/c.rs",
        "x",
        "x/inside.rs",
        "sub/.gitignore",
        "target/new.rs",
    ]
    .iter()
    .map(|p| root.join(p))
    .collect();

    let patched = patch(&root, &base, &changed, &cancel).unwrap();
    let walked = collect(&root, MAX_FILES, &cancel);
    assert_eq!(contents(&patched), contents(&walked));
    assert!(contents(&patched).0.contains(&"source/b.rs"));
    assert!(!contents(&patched).0.contains(&"sub/drop.tmp"));

    // 通知中路径的顺序不同、同一路径出现两次，结果相同。
    let mut shuffled: Vec<PathBuf> = changed.iter().rev().cloned().collect();
    shuffled.extend(changed.iter().take(3).cloned());
    let again = patch(&root, &base, &shuffled, &cancel).unwrap();
    assert_eq!(contents(&again), contents(&walked));

    // 修改根目录的忽略规则：整个重新遍历。
    std::fs::write(root.join(".gitignore"), "target/\n*.log\nlib/\n").unwrap();
    let rules = patch(&root, &walked, &[root.join(".gitignore")], &cancel).unwrap();
    assert!(!contents(&rules).0.iter().any(|p| p.starts_with("lib/")));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn patching_a_layered_index_keeps_its_layers() {
    let root = crate::test_util::scratch("index-patch-layers");
    for path in ["a.txt", "b.txt", "d/c.txt", "d/e/f.txt", "d/e/g.txt"] {
        write(&root, path);
    }
    let cancel = AtomicBool::new(false);
    // 两层（3 个）放得下，三层（5 个）放不下：收录两层，`d/e` 没有遍历过。
    let base = collect(&root, 4, &cancel);
    assert_eq!(base.layers, Some(2));
    assert!(!base.walked("d/e"));

    // 第 3 层的变化忽略；第 2 层的新文件、新目录收录，新目录里更深层的内容不收录。
    write(&root, "d/e/h.txt");
    assert!(patch(&root, &base, &[root.join("d/e/h.txt")], &cancel).is_none());
    write(&root, "d/new.txt");
    write(&root, "n/m/deep.txt");
    write(&root, "n/top.txt");
    let changed = [
        root.join("d/new.txt"),
        root.join("n"),
        root.join("n/top.txt"),
    ];
    let patched = patch(&root, &base, &changed, &cancel).unwrap();
    assert_eq!(patched.layers, Some(2));
    assert_eq!(
        contents(&patched),
        (
            vec!["a.txt", "b.txt", "d/c.txt", "d/new.txt", "n/top.txt"],
            vec!["d", "n"]
        )
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// 回归：外部构建删除输出目录后把另一个目录改名移入时，只有这个目录本身的通知，里面的文件曾因此
/// 从文件表中消失，快速打开搜不到。
#[test]
fn a_directory_replaced_from_outside_is_listed_again() {
    let root = crate::test_util::scratch("index-replaced");
    let outside = crate::test_util::scratch("index-replaced-outside");
    write(&root, "dist/core/old.d.ts");
    write(&root, "src/a.rs");
    write(&outside, "out/core/new.d.ts");
    let base = FileIndex::collect_now(&root);
    let cancel = AtomicBool::new(false);

    // 目录没有变化时（例如目录里的文件被修改时目录本身也有通知）不更新文件表。
    let dist = root.join("dist");
    assert!(patch(&root, &base.files, std::slice::from_ref(&dist), &cancel).is_none());

    std::fs::remove_dir_all(&dist).unwrap();
    std::fs::rename(outside.join("out"), &dist).unwrap();
    let files = patch(&root, &base.files, std::slice::from_ref(&dist), &cancel).unwrap();
    let listed: Vec<&str> = files.paths.iter().collect();
    assert_eq!(listed, ["dist/core/new.d.ts", "src/a.rs"]);
    assert!(files.walked("dist/core"));

    // 上一级目录重新收录过时，同一批里的下一级不重复处理，结果相同。
    let core = dist.join("core");
    let both = patch(&root, &base.files, &[dist.clone(), core], &cancel).unwrap();
    assert_eq!(both.paths.iter().collect::<Vec<_>>(), listed);

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn changes_wait_for_the_running_job_and_are_applied_after_it() {
    let root = crate::test_util::scratch("index-apply");
    write(&root, "a.rs");

    let (sender, woken) = mpsc::channel();
    let mut index = FileIndex::spawn(&root, move || {
        let _ = sender.send(());
    });
    // 第一次建表的结果还没送回时收到变化：先暂存，建好之后再处理。
    write(&root, "b.rs");
    index.apply(Changes {
        paths: vec![root.join("b.rs")],
        rescan: false,
        git: false,
    });
    // 建表时可能已经看到了 `b.rs`，暂存的变化仍会被处理。
    let mut paths = Vec::new();
    while paths != ["a.rs", "b.rs"] || !index.is_ready() {
        woken
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("后台没做完");
        index.poll();
        paths = (0..index.len())
            .map(|i| index.file(i).unwrap().path.to_owned())
            .collect();
    }

    // 只修改了内容：后台检查后文件表不变。
    index.apply(Changes {
        paths: vec![root.join("a.rs")],
        rescan: false,
        git: false,
    });
    woken
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert!(!index.poll());
    assert!(index.is_ready());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn too_many_changes_while_busy_become_one_walk() {
    let root = crate::test_util::scratch("index-rewalk");
    write(&root, "a.rs");

    let (sender, woken) = mpsc::channel();
    let mut index = FileIndex::spawn(&root, move || {
        let _ = sender.send(());
    });
    // 第一次建表的结果还没取走时收到大量变化：不逐个暂存，取走结果后整个重新遍历。
    write(&root, "b.rs");
    let many = (0..MAX_QUEUED)
        .map(|i| root.join(format!("gone{i}.rs")))
        .chain([root.join("b.rs")])
        .collect();
    index.apply(Changes {
        paths: many,
        rescan: false,
        git: false,
    });
    assert!(index.queued.is_empty() && index.rewalk);
    // 已经确定要重新遍历，之后收到的变化不必再记录。
    index.apply(Changes {
        paths: vec![root.join("a.rs")],
        rescan: false,
        git: false,
    });
    assert!(index.queued.is_empty());
    // 后台还在遍历时系统要求重新扫描（编译时持续写入）：不打断它，同样记为完成后重新遍历。
    let cancel = index.cancel.clone().unwrap();
    index.apply(Changes {
        paths: Vec::new(),
        rescan: true,
        git: false,
    });
    assert!(index.rewalk && !cancel.load(Ordering::Relaxed));

    let mut paths = Vec::new();
    while paths != ["a.rs", "b.rs"] || !index.is_ready() {
        woken
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("后台没做完");
        index.poll();
        paths = (0..index.len())
            .map(|i| index.file(i).unwrap().path.to_owned())
            .collect();
    }
    assert!(!index.rewalk);

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn links_to_files_are_listed_and_links_to_directories_are_not_entered() {
    use std::os::unix::fs::symlink;

    let dir = crate::test_util::scratch("index-links");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("lib/a.rs"), "x").unwrap();
    symlink(dir.join("lib/a.rs"), dir.join("link.rs")).unwrap();
    symlink(dir.join("lib"), dir.join("lib-link")).unwrap();
    symlink(dir.join("missing.rs"), dir.join("broken.rs")).unwrap();

    let idx = FileIndex::collect_now(&dir);
    assert_eq!(paths(&idx), ["lib/a.rs", "link.rs"]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn collect_returns_relative_slash_separated_paths() {
    let dir = crate::test_util::scratch("index-rel");
    std::fs::create_dir_all(dir.join("a/b")).unwrap();
    std::fs::write(dir.join("a/b/c.txt"), "x").unwrap();

    let idx = FileIndex::collect_now(&dir);
    assert_eq!(idx.len(), 1);
    assert_eq!(idx.file(0).unwrap().path, "a/b/c.txt");
    assert_eq!(idx.file(0).unwrap().name, "c.txt");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 远端主机上的目录：文件表由远端的一条命令列出，是仓库时遵守其忽略规则，不是仓库时只跳过
/// 文件树中不显示的名字。
#[cfg(unix)]
#[test]
fn files_on_a_remote_host_are_listed_by_one_command_there() {
    use std::os::unix::ffi::OsStrExt;

    let local = crate::test_util::scratch("index-remote");
    for file in [
        "src/main.rs",
        "src/deep/lib.rs",
        "target/out.o",
        "README.md",
        ".git-keep",
    ] {
        let path = local.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }
    std::fs::create_dir_all(local.join("node_modules/pkg")).unwrap();
    std::fs::write(local.join("node_modules/pkg/index.js"), "").unwrap();
    let target = crate::test_util::loopback("index-remote");
    let root = winc_backend::virtual_path(&target, local.as_os_str().as_bytes());
    let listed = |index: &FileIndex| -> Vec<String> {
        (0..index.len())
            .map(|i| index.file(i).unwrap().path.to_owned())
            .collect()
    };

    // 不是仓库：全部文件，一次往返。
    let before = winc_backend::remote::requests(&target);
    let index = FileIndex::collect_now(&root);
    assert_eq!(winc_backend::remote::requests(&target) - before, 2);
    let mut all = listed(&index);
    all.sort();
    assert_eq!(
        all,
        [
            ".git-keep",
            "README.md",
            "node_modules/pkg/index.js",
            "src/deep/lib.rs",
            "src/main.rs",
            "target/out.o"
        ]
    );
    assert_eq!(
        index.absolute(index.position_of(&root.join("src/main.rs")).unwrap()),
        Some(root.join("src/main.rs"))
    );

    // 是仓库：被忽略的文件不收录，未跟踪的照常收录。
    std::fs::write(local.join(".gitignore"), "target/\nnode_modules/\n").unwrap();
    let git = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&local)
        .status();
    if git.is_ok_and(|status| status.success()) {
        let before = winc_backend::remote::requests(&target);
        let index = FileIndex::collect_now(&root);
        assert_eq!(winc_backend::remote::requests(&target) - before, 1);
        let mut kept = listed(&index);
        kept.sort();
        assert_eq!(
            kept,
            [
                ".git-keep",
                ".gitignore",
                "README.md",
                "src/deep/lib.rs",
                "src/main.rs"
            ]
        );
    }
    winc_backend::remote::disconnect(&target);
}
