//! 终端模块的公共部分：主程序和辅助进程 `winc-pty` 都使用。打开伪终端、启动 shell 和事件循环
//! 在可执行文件的入口处实现。
//!
//! - [`term`]：终端仿真，字节流经 `vte` 切分成序列后作用到屏幕网格和回滚缓冲，产出帧和事件。
//! - [`protocol`]：主程序和辅助进程之间的二进制分帧协议，两端使用同一份编解码。
//! - [`width`]：一个字符占几格，主程序绘制格子时必须采用同一套宽度。
//! - [`input`]、[`client`]：编码按键与粘贴，启动辅助进程并收发消息。

mod host;
pub mod protocol;
pub mod scripts;
pub mod term;

pub use host::{client, input};
pub use protocol::{Color, Event, Frame, Line, Marks, Run, Style};
pub use term::{Terminal, width};
