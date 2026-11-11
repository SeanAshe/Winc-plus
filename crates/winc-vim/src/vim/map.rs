//! 用户的映射：按模式把一串按键换成另一串按键或宿主的命令，不递归。
//!
//! 已输入的按键是某条映射的前缀时先保存起来等待后续的按键；宿主在等待超过设定的时间后调用
//! [`Vim::timeout`]。插入模式下等待中的字符先输入到文本里，映射匹配之后再撤回。

use super::{Shared, Vim};
use crate::command::Mode;
use crate::host::{AppCommand, Host, Input, Message};
use crate::key::Key;

/// 映射的右侧。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mapped {
    /// 一串按键，执行时不再经过映射。
    Keys(Vec<Key>),
    /// 宿主的命令名。
    Command(String),
}

/// 一条映射。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    /// 生效的模式：普通、可视、插入或等待动作。
    pub mode: Mode,
    /// 左侧的按键，不为空。
    pub keys: Vec<Key>,
    /// 右侧。
    pub to: Mapped,
}

impl Shared {
    /// 把映射设为 `mappings`；左侧为空的映射被丢弃。
    pub fn set_mappings(&mut self, mappings: Vec<Mapping>) {
        self.mappings = mappings;
        self.mappings.retain(|mapping| !mapping.keys.is_empty());
    }
}

impl Vim {
    /// 是否保存着按键、正在等待映射的后续按键。
    pub fn awaits_mapping(&self) -> bool {
        !self.mapped.is_empty()
    }

    /// 等待映射的后续按键超时：已保存的按键按现有的映射执行，其余的按原样处理。
    pub fn timeout(&mut self, shared: &mut Shared, host: &mut impl Host) {
        self.map_timeout(shared, host);
        self.finish_key(shared, host);
    }

    /// 返回当前可以开始一条映射的模式；正在输入多键命令或其参数时返回 `None`。
    fn map_mode(&self) -> Option<Mode> {
        if self.prompt().is_some() || self.confirm.is_some() || !self.parser.at_start() {
            return None;
        }
        Some(match self.mode {
            Mode::Insert => Mode::Insert,
            _ if self.parser.is_operator_pending() => Mode::OperatorPending,
            mode => mode,
        })
    }

    /// 处理按键 `key`，先经过用户的映射；返回是否已由 Vim 层处理。
    pub(super) fn key_mapped(
        &mut self,
        shared: &mut Shared,
        host: &mut impl Host,
        key: Key,
    ) -> bool {
        if self.mapped.is_empty() {
            let starts = self.map_mode().is_some_and(|mode| {
                let begins = |mapping: &Mapping| mapping.keys.first() == Some(&key);
                (shared.mappings.iter()).any(|mapping| mapping.mode == mode && begins(mapping))
            });
            if !starts {
                return self.key_raw(shared, host, key);
            }
        }
        self.mapped.push((key, false));
        self.map_resolve(shared, host, false);
        true
    }

    /// 已保存的按键不再等待后续按键，立即处理。
    pub(super) fn map_timeout(&mut self, shared: &mut Shared, host: &mut impl Host) {
        if !self.mapped.is_empty() {
            self.map_resolve(shared, host, true);
        }
    }

    /// 处理已保存的按键：能匹配更长的映射且 `expired` 为假时继续等待；否则执行与开头的按键
    /// 匹配的最长的映射，没有时把第一个按键按原样处理，直到没有按键或需要等待为止。
    fn map_resolve(&mut self, shared: &mut Shared, host: &mut impl Host, expired: bool) {
        while !self.mapped.is_empty() {
            let Some(mode) = self.map_mode() else {
                self.map_pass(shared, host);
                continue;
            };
            let typed: Vec<Key> = self.mapped.iter().map(|(key, _)| *key).collect();
            let of_mode = || {
                shared
                    .mappings
                    .iter()
                    .filter(|mapping| mapping.mode == mode)
            };
            let longer = of_mode().any(|mapping| {
                mapping.keys.len() > typed.len() && mapping.keys.starts_with(&typed)
            });
            if longer && !expired {
                // 插入模式下等待中的字符先显示出来。
                if let Some((key, shown)) = self.mapped.last_mut()
                    && mode == Mode::Insert
                    && !*shown
                    && let Some(c) = key.as_char()
                {
                    *shown = true;
                    self.play(host, &Input::Text(c.to_string()));
                }
                return;
            }
            let best = of_mode()
                .filter(|mapping| typed.starts_with(&mapping.keys))
                .max_by_key(|mapping| mapping.keys.len())
                .cloned();
            let Some(mapping) = best else {
                self.map_pass(shared, host);
                continue;
            };
            // 撤回已显示的字符。
            let used: Vec<(Key, bool)> = self.mapped.drain(..mapping.keys.len()).collect();
            for _ in used.iter().filter(|(_, shown)| *shown) {
                self.play(host, &Input::Backspace { word: false });
            }
            match mapping.to {
                Mapped::Keys(keys) => {
                    host.begin_group();
                    self.play_now(shared, host, &keys, false);
                    host.end_group();
                }
                Mapped::Command(name) => {
                    if !host.app(AppCommand::Run(name)) {
                        self.error(host, Message::Unsupported);
                    }
                }
            }
        }
    }

    /// 把保存的第一个按键按原样处理；已显示在文本里的字符不再输入。
    fn map_pass(&mut self, shared: &mut Shared, host: &mut impl Host) {
        let (key, shown) = self.mapped.remove(0);
        if !shown {
            self.type_key(shared, host, key, false);
        }
    }
}
