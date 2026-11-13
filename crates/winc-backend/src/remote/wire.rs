//! 协议的帧和编解码：客户端与服务进程在标准输入输出上交换的字节。
//!
//! 一帧是 `[u32 长度][u8 类型][u32 请求号][载荷]`，整数为小端，长度不含自身的 4 个字节。
//! 应答沿用请求的请求号，以 [`kind::OK`] 或 [`kind::ERROR`] 结束，之前可以有若干
//! [`kind::CHUNK`]；不同请求的帧可以交错。载荷由定长整数和带 `u32` 长度
//! 前缀的字节串顺序排列而成，没有自描述的结构，双方按类型约定的顺序读写。

#[cfg(test)]
mod tests;

use std::io::{self, Read, Write};

use crate::fs::{Kind, Metadata, Stamp, Unix};

/// 协议的版本号。连接建立时双方交换，不一致时不继续：服务进程随安装包发布，版本总是相同，
/// 不一致说明远端的程序不是这一版安装包上传的。
///
/// 只在已发布的版本之后帧的格式或含义有变化时加一，与本程序的版本号无关。
pub(crate) const VERSION: u32 = 1;
/// 一次读取请求最多返回的字节数（字节，1 MiB）。
///
/// 取值由两方面约束：单帧在发送完之前占用连接，过大时其他请求等待的时间变长；过小时读取一个
/// 文件的往返次数增多。1 MiB 时不超过 `LARGE_FILE`（16 MiB）的文件最多 16 次往返。
pub(crate) const MAX_READ: u32 = 1 << 20;
/// 一帧的最大长度（字节，不含长度字段）：一次读取的数据加上 1 KiB 的其余字段。超过时按协议
/// 错误处理并断开，对方无法让本进程按一个任意大的长度分配内存。
pub(crate) const MAX_FRAME: usize = MAX_READ as usize + 1024;
/// 类型和请求号的长度（字节）。
const HEADER: usize = 5;

/// 帧的类型。
pub(crate) mod kind {
    /// 交换协议版本：载荷是 `u32` 版本号，应答相同。
    pub(crate) const HELLO: u8 = 1;
    /// 取文件信息，跟随符号链接：载荷是路径，应答是文件信息。
    pub(crate) const METADATA: u8 = 2;
    /// 取文件信息，不跟随符号链接：载荷是路径，应答是文件信息。
    pub(crate) const SYMLINK_METADATA: u8 = 3;
    /// 列目录：载荷是路径，应答是 `u32` 项数和各项的种类（`u8`）与名字。
    pub(crate) const READ_DIR: u8 = 4;
    /// 读取文件的一段：载荷是路径、`u64` 偏移和 `u32` 字节数，应答是数据和是否已到末尾（`u8`）。
    /// 较长的数据分成几段：前面各段作为 [`CHUNK`] 送回，应答带着最后一段。
    pub(crate) const READ: u8 = 5;
    /// 运行命令：载荷是程序、工作目录（空表示不指定）、`u32` 参数个数和各参数、`u32` 环境变量
    /// 个数和各变量的名字与值、标准输入的内容。运行期间以若干 [`CHUNK`] 送回输出，结束时应答
    /// 是否有退出码（`u8`）和退出码（`u32`，被信号结束时没有）。
    pub(crate) const EXEC: u8 = 6;
    /// 取消请求号相同的 [`EXEC`]：没有载荷，没有应答；被取消的命令以错误结束。
    pub(crate) const CANCEL: u8 = 7;
    /// 读取符号链接的目标：载荷是路径，应答是目标的原始字节。
    pub(crate) const READ_LINK: u8 = 8;
    /// 创建目录：载荷是路径和是否连同各级上级目录一起创建（`u8`），应答为空。
    pub(crate) const CREATE_DIR: u8 = 9;
    /// 新建文件并写入内容，已有条目时不覆盖：载荷是路径和内容，应答为空。
    pub(crate) const CREATE_NEW: u8 = 10;
    /// 覆盖写入文件：载荷是路径和内容，应答为空。
    pub(crate) const WRITE: u8 = 11;
    /// 改名：载荷是原路径和新路径，应答为空。
    pub(crate) const RENAME: u8 = 12;
    /// 复制文件：载荷是源路径和目标路径，应答是复制的字节数（`u64`）。
    pub(crate) const COPY: u8 = 13;
    /// 把符号链接复制为指向同一目标的链接：载荷是源路径和目标路径，应答为空。
    pub(crate) const COPY_SYMLINK: u8 = 14;
    /// 删除文件：载荷是路径，应答为空。
    pub(crate) const REMOVE_FILE: u8 = 15;
    /// 开始一次保存：载荷是路径、是否写入字节序标记（`u8`）、内容是否全部在内存中（`u8`）。
    /// 没有应答，其后是同一请求号的若干 [`SAVE_DATA`] 和一个 [`SAVE_END`]。
    pub(crate) const SAVE_BEGIN: u8 = 16;
    /// 保存的一段内容：载荷是数据。没有应答。
    pub(crate) const SAVE_DATA: u8 = 17;
    /// 结束一次保存：载荷是是否写入（`u8`，为 0 时丢弃已收到的内容）。应答是结果（`u8`，见
    /// [`SAVED`] 等）；为 [`SAVED`] 时其后是写入后的文件信息。
    pub(crate) const SAVE_END: u8 = 18;
    /// 删除文件，或目录连同其中的全部内容：载荷是路径，应答为空。
    pub(crate) const REMOVE_ALL: u8 = 19;
    /// 设置文件的权限位：载荷是路径和权限位（`u32`，只取低 12 位），应答为空。
    pub(crate) const SET_MODE: u8 = 22;
    /// 创建符号链接：载荷是链接指向的目标（原样的写法）和链接的路径，应答为空。
    pub(crate) const SYMLINK: u8 = 23;
    /// 答复运行中的命令提出的输入请求（见 [`ASK`]）：请求号是那条命令的，载荷是请求的编号
    /// （`u32`）、是否给出了答复（`u8`）和答复的内容。没有应答。
    pub(crate) const ANSWER: u8 = 24;
    /// 一个请求的载荷放不进一帧时在前面发出的部分：载荷是其中一段。没有应答；同一请求号随后
    /// 的请求以各段依次相接、再接上它自己的载荷作为完整的载荷。
    pub(crate) const PART: u8 = 20;
    /// 一次往返中的若干个请求：载荷是个数（`u32`），其后每个是类型（`u8`）和它的载荷（字节
    /// 串）。只能包含在读取请求的线程上直接应答的类型（文件信息、读取等），不含运行命令和
    /// 保存。应答按相同的顺序给出每个请求的结果：是否成功（`u8`）和载荷（成功时是它的应答，
    /// 失败时是错误）。各应答合计超过 [`MAX_READ`](super::MAX_READ) 之后的请求不再执行，按
    /// 失败应答。
    pub(crate) const BATCH: u8 = 21;
    /// 探测对端是否还在应答：没有载荷，应答也是空的。
    pub(crate) const PING: u8 = 25;
    /// [`SAVE_END`] 的结果：已保存。
    pub(crate) const SAVED: u8 = 0;
    /// [`SAVE_END`] 的结果：目标文件只读。
    pub(crate) const SAVE_READ_ONLY: u8 = 1;
    /// [`SAVE_END`] 的结果：改名替换会改变文件的身份，而内容无法原地写入。
    pub(crate) const SAVE_IDENTITY: u8 = 2;
    /// [`SAVE_END`] 的结果：无法创建临时文件。
    pub(crate) const SAVE_TEMPORARY: u8 = 3;
    /// 请求成功，载荷是结果。
    pub(crate) const OK: u8 = 0x80;
    /// 请求尚未结束，载荷是结果的一段。运行命令时是输出的一段：来源（`u8`，见 [`STDOUT`]、
    /// [`STDERR`]）和数据；列目录时是一批目录项：项数（`u32`）和各项的种类与名字。
    pub(crate) const CHUNK: u8 = 0x81;
    /// 运行中的命令需要用户输入（口令、是否信任主机）：载荷是请求的编号（`u32`）、提问的
    /// 方式（`ssh` 的 `SSH_ASKPASS_PROMPT`，没有时为空）和提示的文字。对端以 [`ANSWER`] 答复。
    pub(crate) const ASK: u8 = 0x82;
    /// [`CHUNK`] 的来源：标准输出。
    pub(crate) const STDOUT: u8 = 1;
    /// [`CHUNK`] 的来源：标准错误。
    pub(crate) const STDERR: u8 = 2;
    /// 请求失败，载荷是错误的种类（`u8`）和说明。
    pub(crate) const ERROR: u8 = 0xFF;
}

/// 收到的一帧。
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Frame {
    /// 类型，见 [`kind`]。
    pub(crate) kind: u8,
    /// 请求号。
    pub(crate) id: u32,
    /// 载荷。
    pub(crate) payload: Vec<u8>,
}

/// 写出一帧。
///
/// # Errors
///
/// [`kind::EXEC`] 中代表服务进程自身的程序名：对端换成它自己的可执行文件的路径。
///
/// 服务进程的位置只有它自己知道（上传目录、版本摘要）；以参数再次运行自身的功能（监视目录）
/// 用这个名字启动。
pub(crate) const SELF: &[u8] = b"winc:self";

/// 载荷超过 [`MAX_FRAME`] 时返回 [`io::ErrorKind::InvalidInput`]，不写出任何字节；其余与写入相同。
pub(crate) fn write_frame(
    out: &mut impl Write,
    kind: u8,
    id: u32,
    payload: &[u8],
) -> io::Result<()> {
    let len = HEADER + payload.len();
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame too large",
        ));
    }
    let mut header = [0u8; 4 + HEADER];
    header[..4].copy_from_slice(&(len as u32).to_le_bytes());
    header[4] = kind;
    header[5..].copy_from_slice(&id.to_le_bytes());
    out.write_all(&header)?;
    out.write_all(payload)
}

/// 读入一帧；对方在两帧之间正常关闭时返回 `None`。
///
/// # Errors
///
/// 长度不在 [`HEADER`] 到 [`MAX_FRAME`] 之间时返回 [`io::ErrorKind::InvalidData`]；帧未读完对方
/// 就关闭时返回 [`io::ErrorKind::UnexpectedEof`]。
pub(crate) fn read_frame(input: &mut impl Read) -> io::Result<Option<Frame>> {
    let mut len = [0u8; 4];
    // 第一个字节之前结束是正常关闭，之后结束是帧不完整。
    match input.read(&mut len[..1])? {
        0 => return Ok(None),
        _ => input.read_exact(&mut len[1..])?,
    }
    let len = u32::from_le_bytes(len) as usize;
    if !(HEADER..=MAX_FRAME).contains(&len) {
        return Err(invalid("frame length out of range"));
    }
    let mut header = [0u8; HEADER];
    input.read_exact(&mut header)?;
    let mut payload = vec![0; len - HEADER];
    input.read_exact(&mut payload)?;
    Ok(Some(Frame {
        kind: header[0],
        id: u32::from_le_bytes([header[1], header[2], header[3], header[4]]),
        payload,
    }))
}

/// 返回说明为 `message` 的协议错误。
pub(crate) fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// 按顺序写出载荷的各个字段。
#[derive(Default)]
pub(crate) struct Writer(Vec<u8>);

impl Writer {
    pub(crate) fn u8(mut self, value: u8) -> Self {
        self.0.push(value);
        self
    }

    /// 写出一个只取 0 或 1 的字节。
    pub(crate) fn flag(mut self, value: bool) -> Self {
        self.0.push(u8::from(value));
        self
    }

    pub(crate) fn u32(mut self, value: u32) -> Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub(crate) fn u64(mut self, value: u64) -> Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    /// 写出带长度前缀的字节串。字节串的长度受 [`MAX_FRAME`] 约束，远小于 `u32` 的范围。
    pub(crate) fn bytes(mut self, value: &[u8]) -> Self {
        self.0
            .extend_from_slice(&(value.len() as u32).to_le_bytes());
        self.0.extend_from_slice(value);
        self
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.0
    }
}

/// 按顺序读出载荷的各个字段；字段不完整时返回 [`io::ErrorKind::InvalidData`]。
pub(crate) struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    pub(crate) fn new(payload: &'a [u8]) -> Self {
        Self(payload)
    }

    fn take(&mut self, len: usize) -> io::Result<&'a [u8]> {
        if self.0.len() < len {
            return Err(invalid("payload truncated"));
        }
        let (head, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(head)
    }

    pub(crate) fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u32(&mut self) -> io::Result<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub(crate) fn u64(&mut self) -> io::Result<u64> {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(bytes))
    }

    pub(crate) fn bytes(&mut self) -> io::Result<&'a [u8]> {
        let len = self.u32()? as usize;
        self.take(len)
    }

    /// 确认载荷已读完：多出的字节说明双方对该类型的约定不一致。
    pub(crate) fn finish(self) -> io::Result<()> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(invalid("payload has trailing bytes"))
        }
    }
}

/// 把文件信息写入 `out`。
#[cfg(any(unix, test))]
pub(crate) fn write_metadata(out: Writer, metadata: &Metadata) -> Writer {
    let stamp = metadata.modified.unwrap_or(Stamp { secs: 0, nanos: 0 });
    let unix = metadata.unix.unwrap_or(Unix {
        mode: 0,
        uid: 0,
        nlink: 0,
        dev: 0,
        ino: 0,
    });
    out.u8(kind_byte(metadata.kind))
        .u64(metadata.len)
        .u8(u8::from(metadata.modified.is_some()))
        .u64(stamp.secs.cast_unsigned())
        .u32(stamp.nanos)
        .u8(u8::from(metadata.readonly))
        .u8(u8::from(metadata.unix.is_some()))
        .u32(unix.mode)
        .u32(unix.uid)
        .u64(unix.nlink)
        .u64(unix.dev)
        .u64(unix.ino)
}

/// 从 `input` 读出文件信息。
pub(crate) fn read_metadata(input: &mut Reader<'_>) -> io::Result<Metadata> {
    let kind = read_kind(input.u8()?)?;
    let len = input.u64()?;
    let has_modified = input.u8()? != 0;
    let stamp = Stamp {
        secs: input.u64()?.cast_signed(),
        nanos: input.u32()?,
    };
    let readonly = input.u8()? != 0;
    let has_unix = input.u8()? != 0;
    let unix = Unix {
        mode: input.u32()?,
        uid: input.u32()?,
        nlink: input.u64()?,
        dev: input.u64()?,
        ino: input.u64()?,
    };
    if stamp.nanos >= 1_000_000_000 {
        return Err(invalid("nanoseconds out of range"));
    }
    Ok(Metadata {
        kind,
        len,
        modified: has_modified.then_some(stamp),
        readonly,
        unix: has_unix.then_some(unix),
    })
}

/// 把种类写成一个字节。
#[cfg(any(unix, test))]
pub(crate) fn kind_byte(kind: Kind) -> u8 {
    match kind {
        Kind::File => 0,
        Kind::Dir => 1,
        Kind::Symlink => 2,
        Kind::Other => 3,
    }
}

/// 从一个字节读出种类。
pub(crate) fn read_kind(byte: u8) -> io::Result<Kind> {
    match byte {
        0 => Ok(Kind::File),
        1 => Ok(Kind::Dir),
        2 => Ok(Kind::Symlink),
        3 => Ok(Kind::Other),
        _ => Err(invalid("unknown file kind")),
    }
}

/// 在协议中传递的错误种类，下标即编码；不在表中的种类按第 0 项传递。
const ERROR_KINDS: [io::ErrorKind; 13] = [
    io::ErrorKind::Other,
    io::ErrorKind::NotFound,
    io::ErrorKind::PermissionDenied,
    io::ErrorKind::AlreadyExists,
    io::ErrorKind::NotADirectory,
    io::ErrorKind::IsADirectory,
    io::ErrorKind::InvalidInput,
    io::ErrorKind::InvalidData,
    io::ErrorKind::Unsupported,
    io::ErrorKind::DirectoryNotEmpty,
    io::ErrorKind::ReadOnlyFilesystem,
    io::ErrorKind::StorageFull,
    io::ErrorKind::Interrupted,
];

/// 把错误编码为 [`kind::ERROR`] 的载荷：种类和说明。
#[cfg(any(unix, test))]
pub(crate) fn encode_error(error: &io::Error) -> Vec<u8> {
    let code = ERROR_KINDS
        .iter()
        .position(|kind| *kind == error.kind())
        .unwrap_or(0);
    let message = error.to_string();
    // 说明只用于显示，过长时截断到 512 字节以内的字符边界。
    let mut end = message.len().min(512);
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    Writer::default()
        .u8(code as u8)
        .bytes(&message.as_bytes()[..end])
        .finish()
}

/// 把 [`kind::ERROR`] 的载荷还原为错误；载荷不合约定时返回协议错误。
pub(crate) fn decode_error(payload: &[u8]) -> io::Error {
    let mut input = Reader::new(payload);
    let parsed = (|| {
        let code = usize::from(input.u8()?);
        let message = String::from_utf8_lossy(input.bytes()?).into_owned();
        Ok::<_, io::Error>((code, message))
    })();
    match parsed {
        Ok((code, message)) => {
            let kind = ERROR_KINDS
                .get(code)
                .copied()
                .unwrap_or(io::ErrorKind::Other);
            io::Error::new(kind, message)
        }
        Err(error) => error,
    }
}
