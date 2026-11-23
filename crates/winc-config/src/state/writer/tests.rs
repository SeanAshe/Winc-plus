use std::sync::mpsc;

use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("winc-state-writer-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.json"));
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn changes_are_written_in_the_order_they_were_submitted() {
    let path = scratch("order");
    let (sender, receiver) = mpsc::channel();
    for index in 0..20 {
        let sender = sender.clone();
        let message = format!("第 {index} 条");
        let change = move |state: &mut State| state.remember_commit_message(&message);
        update(
            path.clone(),
            State::default(),
            change,
            move |state, result| {
                assert!(result.is_ok());
                let _ = sender.send(state.commit_messages.len());
            },
        );
    }
    flush();
    // 每个修改的回调都调用了；合并写入的几个看到的是同一份结果，因此条数只增不减。
    let seen: Vec<usize> = receiver.try_iter().collect();
    assert_eq!(seen.len(), 20);
    assert!(seen.is_sorted() && seen.last() == Some(&20), "{seen:?}");
    let stored = State::load_from(&path);
    assert_eq!(stored.commit_messages.len(), 20);
    assert_eq!(stored.commit_messages[0], "第 19 条");
    assert_eq!(stored.commit_messages[19], "第 0 条");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_reload_sees_the_changes_submitted_before_it() {
    let path = scratch("reload");
    let change = |state: &mut State| state.commit_reformat = true;
    update(path.clone(), State::default(), change, |_, _| {});
    let (sender, receiver) = mpsc::channel();
    reload(path.clone(), move |state| {
        let _ = sender.send(state.commit_reformat);
    });
    flush();
    assert_eq!(receiver.try_recv(), Ok(true));
    let _ = std::fs::remove_file(&path);
}
