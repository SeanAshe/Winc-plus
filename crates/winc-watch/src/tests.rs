//! `winc-watch` 的测试：判断通知要做的事，以及重新扫描请求的间隔。

use super::*;

use notify::event::{AccessKind, CreateKind, Flag};
use notify::{Error, Event};

fn event(kind: EventKind, paths: &[&str]) -> Event {
    paths.iter().fold(Event::new(kind), |event, path| {
        event.add_path(PathBuf::from(path))
    })
}

#[test]
fn each_changed_path_is_one_message() {
    let created = event(EventKind::Create(CreateKind::File), &["/a b/c", "/d"]);
    assert_eq!(
        classify(&Ok(created)),
        Outcome::Send(b"P/a b/c\0P/d\0".to_vec())
    );
}

#[test]
fn reading_a_file_is_not_a_change() {
    let read = event(EventKind::Access(AccessKind::Any), &["/a"]);
    assert_eq!(classify(&Ok(read)), Outcome::Send(Vec::new()));
}

#[test]
fn lost_notifications_and_errors_ask_for_a_rescan() {
    let lost = Event::new(EventKind::Other).set_flag(Flag::Rescan);
    assert_eq!(classify(&Ok(lost)), Outcome::Rescan);
    assert_eq!(classify(&Err(Error::generic("x"))), Outcome::Rescan);
}

#[test]
fn running_out_of_watches_is_reported_as_the_limit() {
    let error = Error::new(ErrorKind::MaxFilesWatch);
    assert_eq!(classify(&Err(error)), Outcome::Limit);
}

#[test]
fn rescans_keep_their_interval() {
    let now = Instant::now();
    assert_eq!(rescan_delay(None, now), Duration::ZERO);
    assert_eq!(rescan_delay(Some(now), now), RESCAN_INTERVAL);
    let second = now + Duration::from_secs(1);
    assert_eq!(
        rescan_delay(Some(now), second),
        RESCAN_INTERVAL - Duration::from_secs(1)
    );
    assert_eq!(
        rescan_delay(Some(now), now + RESCAN_INTERVAL),
        Duration::ZERO
    );
}
