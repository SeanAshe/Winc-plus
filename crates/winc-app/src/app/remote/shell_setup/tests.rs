//! 主机准备状态的登记与结束。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

#[test]
fn a_host_is_pending_until_every_setup_ends() {
    let target = Target::new("me@pending-host").unwrap();
    let ended = Arc::new(AtomicUsize::new(0));
    let begin = || {
        let ended = Arc::clone(&ended);
        let done: Box<dyn Fn() + Send> = Box::new(move || {
            ended.fetch_add(1, Ordering::Relaxed);
        });
        Setup::begin(&target, Some(done))
    };
    assert!(!shell_pending(&target));
    let (first, second) = (begin(), begin());
    drop(first);
    assert!(shell_pending(&target));
    assert_eq!(ended.load(Ordering::Relaxed), 1);
    // 线程 panic 时同样结束。
    let worker = std::thread::spawn(move || {
        let _setup = second;
        panic!("准备中途退出");
    });
    assert!(worker.join().is_err());
    assert!(!shell_pending(&target));
    assert_eq!(ended.load(Ordering::Relaxed), 2);
}
