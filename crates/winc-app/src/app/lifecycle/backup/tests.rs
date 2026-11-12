//! 热退出备份的测试：备份间隔随内容大小放大，超过上限时封顶。

use super::*;

impl App {
    /// 测试用：本窗口是否持有备份锁。
    pub(in crate::app) fn holds_backup_lock(&self) -> bool {
        self.backup.lock.as_ref().is_some_and(|l| l.1.is_some())
    }

    /// 测试用：清除已排定的备份到期时刻，下一次 [`App::backup_tick`] 重新排定。
    pub(in crate::app) fn forget_backup_due(&mut self) {
        self.backup.due = None;
    }
}

#[test]
fn large_contents_are_backed_up_less_often() {
    // 小的内容按 1 秒的间隔写一次；越大等得越久，64 MiB 约 8.5 秒，再大也不超过 30 秒。
    assert_eq!(backup_delay(0), BACKUP_DELAY);
    assert_eq!(backup_delay(BACKUP_SMALL), BACKUP_DELAY);
    let big = backup_delay(64 << 20);
    assert!(big > Duration::from_secs(8) && big < Duration::from_secs(9));
    assert_eq!(backup_delay(1 << 30), BACKUP_MAX_DELAY);
    assert!(backup_delay(16 << 20) < backup_delay(32 << 20));
}

/// 回归：先写新备份再删旧备份，路径变化的文档在两步之间中止时仍保留旧备份；删除时跳过刚
/// 写好的文件和这次写失败的文档的旧备份。
#[test]
fn stale_backups_are_removed_only_when_safe() {
    let dir = std::env::temp_dir().join(format!("winc-stale-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = |name: &str| {
        let path = dir.join(name);
        std::fs::write(&path, "x").unwrap();
        path
    };
    let (reused, kept, gone) = (file("reused"), file("kept"), file("gone"));
    let removes = vec![
        // 关闭的标签的旧备份，与新打开的标签刚写的备份同名。
        (DocumentId(1), reused.clone()),
        // 改了路径、新备份没写成的文档：旧备份是该文档仅有的备份。
        (DocumentId(2), kept.clone()),
        (DocumentId(3), gone.clone()),
    ];
    let failed = vec![(DocumentId(2), "磁盘已满".to_owned())];
    remove_stale(removes, &[reused.as_path()], &failed);
    assert!(reused.exists() && kept.exists());
    assert!(!gone.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}
