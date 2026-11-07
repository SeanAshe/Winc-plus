//! 各种消息的编码和解码：消息类型的编号，以及 `ToHelper`、`FromHelper` 的 `Message` 实现。

use super::*;

pub(super) mod kind {
    //! 消息类型号：写在线上的第一个字节，已有的编号不得改动。

    /// 新建终端。
    pub const SPAWN: u8 = 1;
    /// 写进伪终端的输入。
    pub const INPUT: u8 = 2;
    /// 尺寸变化。
    pub const RESIZE: u8 = 3;
    /// 主程序完成一帧绘制，可以继续发送。
    pub const CREDIT: u8 = 4;
    /// 取回滚缓冲里的行。
    pub const HISTORY: u8 = 5;
    /// 清屏幕和回滚缓冲。
    pub const CLEAR: u8 = 6;
    /// 关掉终端。
    pub const CLOSE: u8 = 7;
    /// 主题变化。
    pub const THEME: u8 = 8;
    /// 结束程序，保留终端。
    pub const STOP: u8 = 9;

    /// 握手，辅助进程启动后的第一条消息。
    pub const HELLO: u8 = 64;
    /// 屏幕变化。
    pub const FRAME: u8 = 65;
    /// 对历史请求的回答。
    ///
    /// 线协议上的名字是 `LINES`，对应 `FromHelper::History`；编号已经固定，不能按消息种类改名。
    pub const LINES: u8 = 66;
    /// 终端事件。
    pub const EVENT: u8 = 67;
    /// 前台进程变化。
    pub const PROCESS: u8 = 68;
    /// shell 退出。
    pub const EXITED: u8 = 69;
    /// 新建终端失败。
    pub const FAILED: u8 = 70;
    /// 命令块变化。
    pub const BLOCKS: u8 = 71;
    /// 伪终端断开。
    pub const DISCONNECTED: u8 = 72;
}

impl Message for ToHelper {
    fn encode(&self, out: &mut Vec<u8>) {
        let mut w = Writer::begin(out);

        match self {
            ToHelper::Spawn {
                tab,
                program,
                args,
                login,
                cwd,
                env,
                scrollback,
                size,
                integration,
                banner,
            } => {
                w.u8(kind::SPAWN);
                w.u16(*tab);
                w.bool(program.is_some());
                w.bytes(program.as_deref().unwrap_or_default());
                w.u32(args.len() as u32);
                for arg in args {
                    w.bytes(arg);
                }
                w.bool(*login);
                w.bytes(cwd);
                w.u32(env.len() as u32);
                for (key, value) in env {
                    w.bytes(key);
                    w.bytes(value);
                }
                w.u32(*scrollback);
                w.size(*size);
                w.bool(integration.is_some());
                w.bytes(integration.as_deref().unwrap_or_default());
                w.bytes(banner);
            }
            ToHelper::Input { tab, bytes, rest } => {
                w.u8(kind::INPUT);
                w.u16(*tab);
                w.bytes(bytes);
                w.u64(*rest);
            }
            ToHelper::Resize { tab, size } => {
                w.u8(kind::RESIZE);
                w.u16(*tab);
                w.size(*size);
            }
            ToHelper::Credit { tab } => {
                w.u8(kind::CREDIT);
                w.u16(*tab);
            }
            ToHelper::History {
                tab,
                id,
                start,
                end,
            } => {
                w.u8(kind::HISTORY);
                w.u16(*tab);
                w.u32(*id);
                w.u64(*start);
                w.u64(*end);
            }
            ToHelper::Clear { tab } => {
                w.u8(kind::CLEAR);
                w.u16(*tab);
            }
            ToHelper::Close { tab } => {
                w.u8(kind::CLOSE);
                w.u16(*tab);
            }
            ToHelper::Stop { tab } => {
                w.u8(kind::STOP);
                w.u16(*tab);
            }
            ToHelper::Theme(theme) => {
                w.u8(kind::THEME);
                w.raw(&theme.fg);
                w.raw(&theme.bg);
                w.raw(&theme.cursor);
                for color in &theme.palette {
                    w.raw(color);
                }
            }
        }
        w.finish();
    }

    fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(data);

        let message = match r.u8()? {
            kind::SPAWN => {
                let tab = r.u16()?;
                let has_program = r.bool()?;
                let program = r.bytes()?.to_vec();
                let args = (0..r.count()?)
                    .map(|_| r.bytes().map(<[u8]>::to_vec))
                    .collect::<Result<_, _>>()?;
                let login = r.bool()?;
                let cwd = r.bytes()?.to_vec();
                let env = (0..r.count()?)
                    .map(|_| Ok((r.bytes()?.to_vec(), r.bytes()?.to_vec())))
                    .collect::<Result<_, _>>()?;
                ToHelper::Spawn {
                    tab,
                    program: has_program.then_some(program),
                    args,
                    login,
                    cwd,
                    env,
                    scrollback: r.u32()?,
                    size: r.size()?,
                    integration: {
                        let has = r.bool()?;
                        let dir = r.bytes()?.to_vec();
                        has.then_some(dir)
                    },
                    banner: r.bytes()?.to_vec(),
                }
            }
            kind::INPUT => ToHelper::Input {
                tab: r.u16()?,
                bytes: r.bytes()?.to_vec(),
                rest: r.u64()?,
            },
            kind::RESIZE => ToHelper::Resize {
                tab: r.u16()?,
                size: r.size()?,
            },
            kind::CREDIT => ToHelper::Credit { tab: r.u16()? },
            kind::HISTORY => ToHelper::History {
                tab: r.u16()?,
                id: r.u32()?,
                start: r.u64()?,
                end: r.u64()?,
            },
            kind::CLEAR => ToHelper::Clear { tab: r.u16()? },
            kind::CLOSE => ToHelper::Close { tab: r.u16()? },
            kind::STOP => ToHelper::Stop { tab: r.u16()? },
            kind::THEME => {
                let fg = r.rgb()?;
                let bg = r.rgb()?;
                let cursor = r.rgb()?;
                let mut palette = [[0; 3]; 16];
                for color in &mut palette {
                    *color = r.rgb()?;
                }
                ToHelper::Theme(Theme {
                    fg,
                    bg,
                    cursor,
                    palette,
                })
            }
            _ => return Err(DecodeError("未知的消息类型")),
        };
        r.end()?;
        Ok(message)
    }
}

impl Message for FromHelper {
    fn encode(&self, out: &mut Vec<u8>) {
        let mut w = Writer::begin(out);

        match self {
            FromHelper::Hello {
                magic,
                version,
                build,
            } => {
                w.u8(kind::HELLO);
                w.raw(magic);
                w.u32(*version);
                w.bytes(build.as_bytes());
            }
            FromHelper::Frame { tab, frame } => {
                w.u8(kind::FRAME);
                w.u16(*tab);
                w.u16(frame.cols);
                w.u16(frame.rows);
                w.bool(frame.full);
                w.u16(frame.scroll);
                w.u32(frame.lines.len() as u32);
                for (row, line) in &frame.lines {
                    w.u16(*row);
                    w.line(line);
                }
                let cursor = frame.cursor;
                w.u16(cursor.row);
                w.u16(cursor.col);
                w.u8(cursor.shape as u8);
                w.bool(cursor.blink);
                w.bool(cursor.visible);
                w.u32(frame.modes.0);
                w.u8(frame.keyboard);
                w.u64(frame.history.0);
                w.u64(frame.history.1);
            }
            FromHelper::History {
                tab,
                id,
                start,
                lines,
                more,
            } => {
                w.u8(kind::LINES);
                w.u16(*tab);
                w.u32(*id);
                w.u64(*start);
                w.u32(lines.len() as u32);
                for line in lines {
                    w.line(line);
                }
                w.bool(*more);
            }
            FromHelper::Event { tab, event } => {
                w.u8(kind::EVENT);
                w.u16(*tab);
                w.event(event);
            }
            FromHelper::Blocks { tab, blocks, early } => {
                w.u8(kind::BLOCKS);
                w.u16(*tab);
                w.u32(blocks.len() as u32);
                for block in blocks {
                    w.block(block);
                }
                w.bool(*early);
            }
            FromHelper::Process { tab, name } => {
                w.u8(kind::PROCESS);
                w.u16(*tab);
                w.bytes(name.as_bytes());
            }
            FromHelper::Exited { tab, code } => {
                w.u8(kind::EXITED);
                w.u16(*tab);
                w.i32(*code);
            }
            FromHelper::Disconnected { tab } => {
                w.u8(kind::DISCONNECTED);
                w.u16(*tab);
            }
            FromHelper::Failed { tab, message } => {
                w.u8(kind::FAILED);
                w.u16(*tab);
                w.bytes(message.as_bytes());
            }
        }
        w.finish();
    }

    fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(data);

        let message = match r.u8()? {
            kind::HELLO => FromHelper::Hello {
                magic: *r.take(8)?.first_chunk().ok_or(DecodeError("魔数不完整"))?,
                version: r.u32()?,
                build: r.string()?,
            },
            kind::FRAME => {
                let tab = r.u16()?;
                let cols = r.u16()?;
                let rows = r.u16()?;
                let full = r.bool()?;
                let scroll = r.u16()?;
                let lines = r.list(|r| Ok((r.u16()?, r.line()?)))?;
                let cursor = Cursor {
                    row: r.u16()?,
                    col: r.u16()?,
                    shape: match r.u8()? {
                        0 => CursorShape::Block,
                        1 => CursorShape::Underline,
                        2 => CursorShape::Bar,
                        _ => return Err(DecodeError("光标形状无效")),
                    },
                    blink: r.bool()?,
                    visible: r.bool()?,
                };
                FromHelper::Frame {
                    tab,
                    frame: Frame {
                        cols,
                        rows,
                        full,
                        scroll,
                        lines,
                        cursor,
                        modes: Modes(r.u32()?),
                        keyboard: r.u8()?,
                        history: (r.u64()?, r.u64()?),
                    },
                }
            }
            kind::LINES => FromHelper::History {
                tab: r.u16()?,
                id: r.u32()?,
                start: r.u64()?,
                lines: r.list(Reader::line)?,
                more: r.bool()?,
            },
            kind::EVENT => FromHelper::Event {
                tab: r.u16()?,
                event: r.event()?,
            },
            kind::BLOCKS => FromHelper::Blocks {
                tab: r.u16()?,
                blocks: r.list(Reader::block)?,
                early: r.bool()?,
            },
            kind::PROCESS => FromHelper::Process {
                tab: r.u16()?,
                name: r.string()?,
            },
            kind::EXITED => FromHelper::Exited {
                tab: r.u16()?,
                code: r.i32()?,
            },
            kind::DISCONNECTED => FromHelper::Disconnected { tab: r.u16()? },
            kind::FAILED => FromHelper::Failed {
                tab: r.u16()?,
                message: r.string()?,
            },
            _ => return Err(DecodeError("未知的消息类型")),
        };
        r.end()?;
        Ok(message)
    }
}
