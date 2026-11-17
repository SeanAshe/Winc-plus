//! Secret Service（`org.freedesktop.secrets`）中的条目，经 D-Bus 的会话总线访问。
//!
//! 会话使用 `plain` 算法：内容在会话总线上不另行加密，总线只有当前用户的进程可以连接。条目带
//! `service`、`name` 两个属性，保存在默认的密钥环中。密钥环被锁住时由该服务显示解锁的提示。

use std::collections::HashMap;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use super::SERVICE;

const BUS: &str = "org.freedesktop.secrets";
const ROOT: &str = "/org/freedesktop/secrets";
/// 没有对象时接口返回的路径。
const NONE: &str = "/";

/// 接口中的一份秘密：会话、算法参数、内容和内容类型。
type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

/// 到 Secret Service 的连接和已打开的会话。
struct Session {
    connection: Connection,
    path: OwnedObjectPath,
}

impl Session {
    fn open() -> zbus::Result<Session> {
        let connection = Connection::session()?;
        let service = proxy(&connection, ROOT, "Service")?;
        let (_, path): (OwnedValue, OwnedObjectPath) =
            service.call("OpenSession", &("plain", Value::from("")))?;
        Ok(Session { connection, path })
    }

    fn service(&self) -> zbus::Result<Proxy<'_>> {
        proxy(&self.connection, ROOT, "Service")
    }

    /// 返回属性为 `attributes` 的条目；被锁住的条目先解锁，用户拒绝解锁的不返回。
    fn search(&self, attributes: &HashMap<&str, &str>) -> zbus::Result<Vec<OwnedObjectPath>> {
        let (mut items, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
            self.service()?.call("SearchItems", &(attributes,))?;
        if !locked.is_empty() {
            items.extend(self.unlock(&locked)?);
        }
        Ok(items)
    }

    /// 解锁 `objects`，返回已解锁的对象。
    fn unlock(&self, objects: &[OwnedObjectPath]) -> zbus::Result<Vec<OwnedObjectPath>> {
        let (mut unlocked, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) =
            self.service()?.call("Unlock", &(objects,))?;
        if let Some(result) = self.prompt(&prompt)? {
            let more = Vec::<OwnedObjectPath>::try_from(result).unwrap_or_default();
            unlocked.extend(more);
        }
        Ok(unlocked)
    }

    /// 显示提示 `prompt` 并等待用户答复，返回提示的结果；没有提示或用户取消时返回 `None`。
    fn prompt(&self, prompt: &OwnedObjectPath) -> zbus::Result<Option<OwnedValue>> {
        if prompt.as_str() == NONE {
            return Ok(None);
        }
        let prompt = proxy(&self.connection, prompt.as_str(), "Prompt")?;
        let mut completed = prompt.receive_signal("Completed")?;
        // 参数是父窗口的标识；不提供时提示不依附于窗口。
        prompt.call::<_, _, ()>("Prompt", &("",))?;
        let Some(message) = completed.next() else {
            return Ok(None);
        };
        let (dismissed, result): (bool, OwnedValue) = message.body().deserialize()?;
        Ok((!dismissed).then_some(result))
    }

    fn secret(&self, item: &OwnedObjectPath) -> zbus::Result<String> {
        let item = proxy(&self.connection, item.as_str(), "Item")?;
        let (_, _, value, _): Secret = item.call("GetSecret", &(&self.path,))?;
        String::from_utf8(value).map_err(|e| zbus::Error::Failure(e.to_string()))
    }

    /// 在默认的密钥环中创建条目，替换属性相同的条目。
    fn create(&self, name: &str, secret: &str) -> zbus::Result<()> {
        let collection: OwnedObjectPath = self.service()?.call("ReadAlias", &("default",))?;
        if collection.as_str() == NONE {
            return Err(zbus::Error::Failure("no default collection".to_owned()));
        }
        let label = format!("{SERVICE}: {name}");
        let mut properties: HashMap<&str, Value<'_>> = HashMap::new();
        properties.insert("org.freedesktop.Secret.Item.Label", Value::from(label));
        properties.insert(
            "org.freedesktop.Secret.Item.Attributes",
            Value::from(attributes(Some(name))),
        );
        let secret = (
            ObjectPath::from(&self.path),
            Vec::<u8>::new(),
            secret.as_bytes(),
            "text/plain",
        );
        let create = |collection: &OwnedObjectPath| -> zbus::Result<OwnedObjectPath> {
            let collection = proxy(&self.connection, collection.as_str(), "Collection")?;
            let (_, prompt): (OwnedObjectPath, OwnedObjectPath) =
                collection.call("CreateItem", &(&properties, &secret, true))?;
            Ok(prompt)
        };
        // 密钥环被锁住时返回提示：用户解锁后条目即已创建，取消时提示没有结果。
        let prompt = create(&collection)?;
        if prompt.as_str() != NONE && self.prompt(&prompt)?.is_none() {
            return Err(zbus::Error::Failure("dismissed".to_owned()));
        }
        Ok(())
    }
}

fn proxy<'a>(connection: &Connection, path: &'a str, interface: &str) -> zbus::Result<Proxy<'a>> {
    let interface = format!("org.freedesktop.Secret.{interface}");
    Proxy::new(connection, BUS, path, interface)
}

/// 返回条目的属性；`name` 为 `None` 时匹配全部条目。
fn attributes(name: Option<&str>) -> HashMap<&str, &str> {
    let mut attributes = HashMap::from([("service", SERVICE)]);
    if let Some(name) = name {
        attributes.insert("name", name);
    }
    attributes
}

pub(super) fn available() -> bool {
    Session::open().is_ok()
}

pub(super) fn get(name: &str) -> Option<String> {
    let session = Session::open().ok()?;
    let items = session.search(&attributes(Some(name))).ok()?;
    session.secret(items.first()?).ok()
}

pub(super) fn set(name: &str, secret: &str) -> bool {
    Session::open().is_ok_and(|session| session.create(name, secret).is_ok())
}

pub(super) fn delete(name: &str) {
    let Ok(session) = Session::open() else {
        return;
    };
    for item in session.search(&attributes(Some(name))).unwrap_or_default() {
        if let Ok(item) = proxy(&session.connection, item.as_str(), "Item") {
            let _ = item.call::<_, _, OwnedObjectPath>("Delete", &());
        }
    }
}

pub(super) fn names() -> Vec<String> {
    let Ok(session) = Session::open() else {
        return Vec::new();
    };
    let items = session.search(&attributes(None)).unwrap_or_default();
    items
        .iter()
        .filter_map(|item| {
            let item = proxy(&session.connection, item.as_str(), "Item").ok()?;
            let mut found: HashMap<String, String> = item.get_property("Attributes").ok()?;
            found.remove("name")
        })
        .collect()
}
