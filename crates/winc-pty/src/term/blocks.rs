//! 命令块：从 shell 集成记在行上的标记里划分出一条条命令。
//!
//! 标记（OSC 133 的 A、B、C、D）记在主屏的行上，随行滚入回滚缓冲、随重排移动位置，因此块的
//! 位置不单独记录，每次发送时从行上的标记重新数一遍。标记只在 shell 画提示符和运行命令时出现，
//! 刷屏时重数不频繁。行上只有块的编号，命令行、目录、运行时长按编号记在 [`Blocks`] 里。

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;

use crate::protocol::{Block, Marks};

/// 一条命令行记录的最大长度（字节，64 KiB）。
///
/// 块每次变化时所有块连同命令行整体发给主程序，数兆字节的命令留在回滚缓冲里会让消息超过协议
/// 上限、连接断开，超出该长度的部分截断。
const MAX_TEXT: usize = 64 * 1024;

/// 一次发送的块编码后的总大小上限（字节，[`Block::encoded_len`] 之和，8 MiB）。
///
/// 块每次变化都整体重发，包括已滚入回滚缓冲的块，消息超过协议上限会断开连接。超出时从最旧的
/// 块起不再记录命令行和目录，仍然超出时不再发送最旧的块。
const MAX_BYTES: usize = 8 << 20;

/// 一个块中不记在行上的信息。
#[derive(Debug, Default)]
struct Info {
    /// shell 报的命令行。
    text: String,
    /// 开始运行时的当前目录。
    cwd: Rc<[u8]>,
    /// 开始运行（收到 `C`）的时刻。
    started: Option<Instant>,
    /// 运行时长（毫秒），结束时计算一次。
    duration: Option<u32>,
}

/// 一个终端的命令块。
#[derive(Debug, Default)]
pub(super) struct Blocks {
    /// 下一个块的编号。
    next_id: u32,
    /// 最近一个提示符开始的块。
    current: Option<u32>,
    /// 按块编号记着不在行上的信息。
    infos: HashMap<u32, Info>,
    /// 最近一次 OSC 7 报告的目录。
    ///
    /// 各个块共用同一份，程序连续输出数万个提示符时，长路径不会被复制数万份。
    pub cwd: Rc<[u8]>,
    /// 标记是否已变化（新标记、重排、清空回滚缓冲），为真时下次取出重新计数。
    pub dirty: bool,
    /// 上次看到的屏幕标记指纹，由行号和标记算出，清屏擦掉标记时随之变化。
    signature: u64,
    /// 上次发送的块，内容不变时不重复发送。
    sent: Vec<Block>,
    /// shell 是否停在提示符上，即收到 `A` 之后、`C` 之前。
    pub at_prompt: bool,
    /// shell 是否在终端改宽度时自行重画整个提示符。
    ///
    /// 由 `A` 的 `redraw=1` 参数置位，winc 的 zsh、fish 集成会发送该参数；为真时改宽度先擦掉
    /// 提示符、不重排提示符所在的行（见 `Emulator::clear_prompt`）。
    pub redraws_prompt: bool,
}

impl Blocks {
    /// 开始新提示符对应的块，返回块编号，编号记在提示符所在的行上。
    pub fn prompt(&mut self) -> u32 {
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let id = self.next_id;
        self.current = Some(id);
        self.at_prompt = true;
        self.infos.insert(
            id,
            Info {
                cwd: Rc::clone(&self.cwd),
                ..Info::default()
            },
        );
        self.dirty = true;
        id
    }

    /// 记录命令开始运行（`C`）。
    ///
    /// `text` 是 shell 报告的命令行，超过 [`MAX_TEXT`] 的部分截断；为 `None`（没有报告或不可信）
    /// 时保留已记录的命令行。
    pub fn output(&mut self, text: Option<String>) {
        if let Some(info) = self.current.and_then(|id| self.infos.get_mut(&id)) {
            if let Some(mut text) = text {
                if text.len() > MAX_TEXT {
                    let mut end = MAX_TEXT;
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    text.truncate(end);
                }
                info.text = text;
            }
            info.cwd = Rc::clone(&self.cwd);
            info.started = Some(Instant::now());
        }
        self.at_prompt = false;
        // 该参数只对发送它的提示符有效：命令运行期间可能进入其他 shell（如 ssh），它们不会重画。
        self.redraws_prompt = false;
        self.dirty = true;
    }

    /// 当前命令块的命令是否还在运行。
    pub fn running(&self) -> bool {
        let info = self.current.and_then(|id| self.infos.get(&id));
        info.is_some_and(|info| info.started.is_some())
    }

    /// 记录命令结束（`D`），并算出运行时长。
    pub fn done(&mut self) {
        if let Some(info) = self.current.and_then(|id| self.infos.get_mut(&id))
            && let Some(started) = info.started.take()
        {
            info.duration = Some(started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32);
        }
        self.dirty = true;
    }

    /// 返回全部命令块，与上次取出时相同则返回 `None`，编码后的总大小不超过 [`MAX_BYTES`]。
    ///
    /// - `screen`：屏幕上带标记的行（绝对行号、标记），用于判断标记是否变化。
    /// - `history`：回滚缓冲中带标记的行，需要重新计数时才读取。
    /// - `first`：回滚缓冲的第一行，已发送的最早的块滚出回滚缓冲后需要重新计数。
    pub fn take(
        &mut self,
        first: u64,
        screen: &[(u64, Marks)],
        history: impl Iterator<Item = (u64, Marks)>,
    ) -> Option<Vec<Block>> {
        // FNV-1a 64 位：偏移基与质数取该哈希的标准值，行号与标记打包后逐项异或再乘质数。
        // 该哈希只用于判断标记是否变化，不需要抗碰撞。
        let signature = screen
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, (line, m)| {
                let item = line
                    ^ (u64::from(m.flags) << 56)
                    ^ (u64::from(m.block) << 24)
                    ^ u64::from(m.exit as u32);
                (hash ^ item).wrapping_mul(0x0100_0000_01b3)
            });
        let trimmed = self.sent.first().is_some_and(|b| b.prompt < first);
        if !self.dirty && !trimmed && signature == self.signature {
            return None;
        }
        self.dirty = false;
        self.signature = signature;

        let mut blocks = collect(history.chain(screen.iter().copied()));
        // 从最新的块向前填入命令行、目录和时长，容纳不下的旧块不再记录命令行和目录并释放已记录的
        // 部分；块本身也容纳不下时，更旧的块都不发送。
        let mut bytes = 0;
        let mut first = 0;
        for (i, block) in blocks.iter_mut().enumerate().rev() {
            if let Some(info) = self.infos.get_mut(&block.id) {
                block.duration = info.duration;
                if bytes + block.encoded_len() + info.text.len() + info.cwd.len() <= MAX_BYTES {
                    block.text.clone_from(&info.text);
                    block.cwd = info.cwd.to_vec();
                } else {
                    info.text = String::new();
                    info.cwd = Rc::default();
                }
            }
            bytes += block.encoded_len();
            if bytes > MAX_BYTES {
                first = i + 1;
                break;
            }
        }
        blocks.drain(..first);
        // 不在任何行上的块（已滚出回滚缓冲或被擦除）和不再发送的块不必记录。当前块始终保留：
        // 提示符可能在绘制完成之前就被清屏。
        let current = self.current;
        let alive: HashSet<u32> = blocks.iter().map(|b| b.id).collect();
        self.infos
            .retain(|id, _| Some(*id) == current || alive.contains(id));
        if blocks == self.sent {
            return None;
        }
        self.sent.clone_from(&blocks);
        Some(blocks)
    }
}

/// 按顺序统计标记，返回各个块的位置，命令行、目录和时长由 [`Blocks::take`] 填入。
///
/// 同一行上的多个标记按 C、D、A、B 的顺序处理：没有输出的命令，结束（`D`）和下一个提示符
/// （`A`）常常与开始运行（`C`）落在同一行。
fn collect(marks: impl Iterator<Item = (u64, Marks)>) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    // 记录最后一个块是否报告过 B。没有报告时（多行命令、shell 不报告 B），命令行取开始输出
    // 的前一行。
    let mut has_command = false;
    for (line, m) in marks {
        if m.flags & Marks::OUTPUT != 0
            && let Some(block) = open(&mut blocks)
            && block.output.is_none()
        {
            block.output = Some(line);
            if !has_command {
                block.command = line.saturating_sub(1).max(block.prompt);
            }
        }
        if m.flags & Marks::DONE != 0
            && let Some(block) = open(&mut blocks)
        {
            block.end = Some(line);
            block.exit = Some(m.exit);
        }
        if m.flags & Marks::PROMPT != 0 {
            if let Some(block) = open(&mut blocks) {
                block.end = Some(line);
            }
            blocks.push(Block {
                id: m.block,
                prompt: line,
                command: line,
                ..Block::default()
            });
            has_command = false;
        }
        if m.flags & Marks::COMMAND != 0
            && let Some(block) = open(&mut blocks)
            && block.output.is_none()
        {
            block.command = line;
            has_command = true;
        }
    }
    blocks
}

/// 返回最后一个尚未结束的块。
fn open(blocks: &mut [Block]) -> Option<&mut Block> {
    blocks.last_mut().filter(|b| b.end.is_none())
}
