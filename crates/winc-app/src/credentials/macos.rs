//! macOS 钥匙串中的通用密码条目。

use security_framework::item::{ItemClass, ItemSearchOptions, Limit};
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

use super::SERVICE;

pub(super) fn available() -> bool {
    true
}

pub(super) fn get(name: &str) -> Option<String> {
    let secret = get_generic_password(SERVICE, name).ok()?;
    String::from_utf8(secret).ok()
}

pub(super) fn set(name: &str, secret: &str) -> bool {
    set_generic_password(SERVICE, name, secret.as_bytes()).is_ok()
}

pub(super) fn delete(name: &str) {
    let _ = delete_generic_password(SERVICE, name);
}

pub(super) fn names() -> Vec<String> {
    let found = ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(SERVICE)
        .limit(Limit::All)
        .load_attributes(true)
        .search()
        .unwrap_or_default();
    found
        .iter()
        .filter_map(|item| item.simplify_dict()?.remove("acct"))
        .collect()
}
