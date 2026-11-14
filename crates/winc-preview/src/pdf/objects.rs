//! 读 PDF 文件里的对象：解析对象语法，按交叉引用表（或交叉引用流）找到间接对象，解开压缩的
//! 对象流和流数据。只为读目录服务：不解析内容流，不支持加密的文件。
//!
//! 交叉引用表损坏（偏移不对、缺失）时退回逐字节查找 `n g obj`，大多数修改过、拼接过的文件仍能
//! 读出目录。读不出来时各个查找返回 `None`，从不 panic。

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use miniz_oxide::inflate::{decompress_to_vec_with_limit, decompress_to_vec_zlib_with_limit};

/// 解开一个流最多得到的字节数：目录、交叉引用流、对象流都远小于它；防止伪造的压缩数据撑爆
/// 内存。
const INFLATE_LIMIT: usize = 64 << 20;
/// 嵌套的数组、字典最多几层：正常文件只有几层，防止伪造的文件让解析递归太深。
const NEST_LIMIT: usize = 64;

/// PDF 的对象。
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Obj {
    /// `null`，也用于读不出来的对象。
    Null,
    /// `true`、`false`。
    Bool(bool),
    /// 整数。
    Int(i64),
    /// 实数。
    Real(f64),
    /// 名字（`/Name`，已解开 `#xx`），不含开头的 `/`。
    Name(Vec<u8>),
    /// 字符串（字面的或十六进制的），已解开转义。
    Str(Vec<u8>),
    /// 数组。
    Array(Vec<Obj>),
    /// 字典。
    Dict(Dict),
    /// 间接引用：对象号和代号。
    Ref(u32, u16),
    /// 流：字典和解码前的数据在文件里的位置 `[起, 止)`。
    Stream(Dict, usize, usize),
}

/// PDF 的字典：按出现的顺序保存键值。
pub(super) type Dict = Vec<(Vec<u8>, Obj)>;

impl Obj {
    /// 字典（或流的字典）里键 `key` 的值。
    pub(super) fn get(&self, key: &[u8]) -> Option<&Obj> {
        let dict = match self {
            Obj::Dict(dict) | Obj::Stream(dict, ..) => dict,
            _ => return None,
        };
        dict.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// 整数值；实数取整。
    pub(super) fn int(&self) -> Option<i64> {
        match self {
            Obj::Int(n) => Some(*n),
            Obj::Real(n) => Some(*n as i64),
            _ => None,
        }
    }
}

/// 交叉引用表的一项。
#[derive(Debug, Clone, Copy)]
enum Entry {
    /// 未压缩的对象在文件里的偏移。
    At(usize),
    /// 放在对象流里的对象：对象流的对象号、在其中的序号。
    Packed(u32, usize),
}

/// 一个 PDF 文件：原始字节、交叉引用表和尾部字典。
pub(super) struct File<'a> {
    bytes: &'a [u8],
    xref: HashMap<u32, Entry>,
    /// 尾部字典（或交叉引用流的字典）：其中有 `/Root`、`/Encrypt`。
    pub trailer: Obj,
    /// 解开过的对象流：对象号到（解码后的数据，各对象的起点）。
    streams: RefCell<HashMap<u32, Option<Packed>>>,
}

/// 解开的对象流：解码后的数据和各对象的起点。
type Packed = (Vec<u8>, Vec<usize>);

impl<'a> File<'a> {
    /// 读入 PDF 文件 `bytes` 的交叉引用表；文件不像 PDF 时返回 `None`。
    pub(super) fn open(bytes: &'a [u8]) -> Option<File<'a>> {
        memchr::memmem::find(&bytes[..bytes.len().min(1024)], b"%PDF")?;
        let mut file = File {
            bytes,
            xref: HashMap::new(),
            trailer: Obj::Null,
            streams: RefCell::default(),
        };
        // 交叉引用表读不出来、或者读出来却找不到根对象时，逐字节查找各个对象重建。
        let read = file.read_xref().is_some();
        if !read || file.resolve(file.trailer.get(b"Root")).is_none() {
            file.scan();
        }
        Some(file)
    }

    /// 从文件末尾的 `startxref` 起读交叉引用表，沿 `/Prev` 读完各次增量更新；新的项优先。
    fn read_xref(&mut self) -> Option<()> {
        let tail = self.bytes.len().saturating_sub(4096);
        let at = memchr::memmem::rfind(&self.bytes[tail..], b"startxref")? + tail;
        let mut lexer = Lexer::new(self.bytes, at + b"startxref".len());
        let mut next = usize::try_from(lexer.object(0)?.int()?).ok();
        let mut seen = HashSet::new();
        while let Some(offset) = next.filter(|&o| o < self.bytes.len() && seen.insert(o)) {
            let trailer = match self.bytes[offset..].starts_with(b"xref") {
                true => self.read_table(offset)?,
                false => self.read_stream(offset)?,
            };
            // 同时有交叉引用表和交叉引用流的文件（`/XRefStm`）：流里的项也要读。
            if let Some(stream) = trailer.get(b"XRefStm").and_then(Obj::int) {
                let _ = self.read_stream(usize::try_from(stream).ok()?);
            }
            next = trailer
                .get(b"Prev")
                .and_then(Obj::int)
                .and_then(|p| p.try_into().ok());
            if self.trailer == Obj::Null {
                self.trailer = trailer;
            }
        }
        (self.trailer != Obj::Null).then_some(())
    }

    /// 读 `offset` 处的交叉引用表（`xref` 起头），返回其后的尾部字典。
    fn read_table(&mut self, offset: usize) -> Option<Obj> {
        let mut lexer = Lexer::new(self.bytes, offset + b"xref".len());
        loop {
            lexer.skip_space();
            if lexer.keyword(b"trailer") {
                return lexer.object(0);
            }
            let first = u32::try_from(lexer.object(0)?.int()?).ok()?;
            let count = u32::try_from(lexer.object(0)?.int()?).ok()?;
            for n in first..first.saturating_add(count) {
                let at = lexer.object(0)?.int()?;
                let _generation = lexer.object(0)?;
                lexer.skip_space();
                let used = lexer.peek() == Some(b'n');
                lexer.pos += 1;
                if used && let Ok(at) = usize::try_from(at) {
                    self.xref.entry(n).or_insert(Entry::At(at));
                }
            }
        }
    }

    /// 读 `offset` 处的交叉引用流（PDF 1.5 起），返回它的字典。
    fn read_stream(&mut self, offset: usize) -> Option<Obj> {
        let stream = self.object_at(offset)?;
        let data = self.decode(&stream)?;
        let widths: Vec<usize> = match stream.get(b"W")? {
            Obj::Array(w) => w
                .iter()
                .map(|w| w.int().and_then(|w| w.try_into().ok()))
                .collect::<Option<_>>()?,
            _ => return None,
        };
        let [a, b, c] = widths[..] else {
            return None;
        };
        let size = stream.get(b"Size").and_then(Obj::int).unwrap_or(0);
        let index: Vec<i64> = match stream.get(b"Index") {
            Some(Obj::Array(index)) => index.iter().filter_map(Obj::int).collect(),
            _ => vec![0, size],
        };
        let row = a + b + c;
        let mut rows = data.chunks_exact(row.max(1));
        for &[first, count] in index.as_chunks::<2>().0 {
            let (first, count) = (u32::try_from(first).ok()?, u32::try_from(count).ok()?);
            for n in first..first.saturating_add(count) {
                let Some(row) = rows.next() else {
                    return Some(stream);
                };
                let field = |from: usize, width: usize| {
                    (row[from..from + width].iter()).fold(0usize, |v, &b| v << 8 | usize::from(b))
                };
                // 第一栏宽为 0 时类型默认为 1（未压缩的对象）。
                let kind = if a == 0 { 1 } else { field(0, a) };
                let entry = match kind {
                    1 => Entry::At(field(a, b)),
                    2 => Entry::Packed(u32::try_from(field(a, b)).ok()?, field(a + b, c)),
                    _ => continue,
                };
                self.xref.entry(n).or_insert(entry);
            }
        }
        Some(stream)
    }

    /// 交叉引用表不可用时逐字节查找 `n g obj`，后出现的同号对象优先（增量更新追加在后面）；
    /// 对象流里的对象一并登记。尾部字典取最后一个 `trailer`，或带 `/Root` 的交叉引用流。
    fn scan(&mut self) {
        self.xref.clear();
        let mut objstms = Vec::new();
        for at in memchr::memmem::find_iter(self.bytes, b"obj") {
            let Some(start) = object_start(self.bytes, at) else {
                continue;
            };
            let mut lexer = Lexer::new(self.bytes, start);
            let Some(Obj::Int(n)) = lexer.object(0) else {
                continue;
            };
            let Ok(n) = u32::try_from(n) else {
                continue;
            };
            self.xref.insert(n, Entry::At(start));
            if let Some(object) = self.object_at(start) {
                match object.get(b"Type") {
                    Some(Obj::Name(t)) if t == b"ObjStm" => objstms.push(n),
                    Some(Obj::Name(t)) if t == b"XRef" && object.get(b"Root").is_some() => {
                        self.trailer = object;
                    }
                    _ => {}
                }
            }
        }
        for stm in objstms {
            let count = self.packed(stm).map_or(0, |(_, starts)| starts.len());
            let numbers = self.packed_numbers(stm);
            for (i, n) in numbers.into_iter().enumerate().take(count) {
                self.xref.entry(n).or_insert(Entry::Packed(stm, i));
            }
        }
        if let Some(at) = memchr::memmem::rfind(self.bytes, b"trailer") {
            let mut lexer = Lexer::new(self.bytes, at + b"trailer".len());
            if let Some(trailer @ Obj::Dict(_)) = lexer.object(0) {
                self.trailer = trailer;
            }
        }
    }

    /// 对象 `obj`：间接引用解开为它指向的对象，其余原样返回；找不到时为 `None`。
    pub(super) fn resolve(&self, obj: Option<&Obj>) -> Option<Obj> {
        match obj? {
            Obj::Ref(n, _) => self.object(*n),
            Obj::Null => None,
            other => Some(other.clone()),
        }
    }

    /// 对象号为 `n` 的间接对象。
    pub(super) fn object(&self, n: u32) -> Option<Obj> {
        match *self.xref.get(&n)? {
            Entry::At(at) => self.object_at(at),
            Entry::Packed(stm, i) => {
                let (data, starts) = self.packed(stm)?;
                let start = *starts.get(i)?;
                Lexer::new(&data, start).object(0)
            }
        }
    }

    /// 解开对象流 `stm`：解码后的数据和其中各对象的起点（已加上 `/First`）。
    fn packed(&self, stm: u32) -> Option<(Vec<u8>, Vec<usize>)> {
        if let Some(cached) = self.streams.borrow().get(&stm) {
            return cached.clone();
        }
        let read = || {
            let Entry::At(at) = *self.xref.get(&stm)? else {
                return None;
            };
            let stream = self.object_at(at)?;
            let data = self.decode(&stream)?;
            let first = usize::try_from(stream.get(b"First")?.int()?).ok()?;
            let count = usize::try_from(stream.get(b"N")?.int()?).ok()?;
            let mut lexer = Lexer::new(&data, 0);
            let mut starts = Vec::with_capacity(count.min(4096));
            for _ in 0..count {
                let _number = lexer.object(0)?;
                let offset = usize::try_from(lexer.object(0)?.int()?).ok()?;
                starts.push(first.checked_add(offset)?);
            }
            Some((data, starts))
        };
        let result = read();
        self.streams.borrow_mut().insert(stm, result.clone());
        result
    }

    /// 对象流 `stm` 开头列出的各个对象号。
    fn packed_numbers(&self, stm: u32) -> Vec<u32> {
        let Some(Entry::At(at)) = self.xref.get(&stm).copied() else {
            return Vec::new();
        };
        let Some(stream) = self.object_at(at) else {
            return Vec::new();
        };
        let (Some(data), Some(count)) = (self.decode(&stream), stream.get(b"N").and_then(Obj::int))
        else {
            return Vec::new();
        };
        let mut lexer = Lexer::new(&data, 0);
        let mut numbers = Vec::new();
        for _ in 0..count.clamp(0, 1 << 20) {
            let (Some(Obj::Int(n)), Some(_)) = (lexer.object(0), lexer.object(0)) else {
                break;
            };
            numbers.extend(u32::try_from(n).ok());
        }
        numbers
    }

    /// 文件 `at` 处的间接对象（`n g obj …`）；是流时连同数据的位置。
    fn object_at(&self, at: usize) -> Option<Obj> {
        let mut lexer = Lexer::new(self.bytes, at);
        let (Some(Obj::Int(_)), Some(Obj::Int(_))) = (lexer.object(0), lexer.object(0)) else {
            return None;
        };
        lexer.skip_space();
        if !lexer.keyword(b"obj") {
            return None;
        }
        let object = lexer.object(0)?;
        let Obj::Dict(dict) = object else {
            return Some(object);
        };
        lexer.skip_space();
        if !lexer.keyword(b"stream") {
            return Some(Obj::Dict(dict));
        }
        // `stream` 后面是一个换行（`\r\n` 或 `\n`），数据从它之后开始。
        let mut start = lexer.pos;
        if self.bytes.get(start) == Some(&b'\r') {
            start += 1;
        }
        if self.bytes.get(start) == Some(&b'\n') {
            start += 1;
        }
        let length = dict.iter().find(|(k, _)| k == b"Length").map(|(_, v)| v);
        let length = match length {
            Some(Obj::Ref(n, _)) if self.xref.contains_key(n) => {
                self.object(*n).as_ref().and_then(Obj::int)
            }
            Some(other) => other.int(),
            None => None,
        };
        let end = length
            .and_then(|l| usize::try_from(l).ok())
            .and_then(|l| start.checked_add(l))
            .filter(|&end| end <= self.bytes.len())
            .or_else(|| {
                memchr::memmem::find(&self.bytes[start..], b"endstream").map(|e| start + e)
            })?;
        Some(Obj::Stream(dict, start, end))
    }

    /// 解码流 `stream` 的数据：只支持不压缩和 `/FlateDecode`（含 PNG 预测）。
    pub(super) fn decode(&self, stream: &Obj) -> Option<Vec<u8>> {
        let Obj::Stream(_, start, end) = *stream else {
            return None;
        };
        let raw = self.bytes.get(start..end)?;
        let filters: Vec<&[u8]> = match stream.get(b"Filter") {
            None | Some(Obj::Null) => Vec::new(),
            Some(Obj::Name(name)) => vec![name.as_slice()],
            Some(Obj::Array(names)) => names
                .iter()
                .map(|n| match n {
                    Obj::Name(n) => Some(n.as_slice()),
                    _ => None,
                })
                .collect::<Option<_>>()?,
            _ => return None,
        };
        let mut data = raw.to_vec();
        for filter in filters {
            if filter != b"FlateDecode" && filter != b"Fl" {
                return None;
            }
            data = decompress_to_vec_zlib_with_limit(&data, INFLATE_LIMIT)
                .or_else(|_| decompress_to_vec_with_limit(&data, INFLATE_LIMIT))
                .ok()?;
        }
        let params = match stream.get(b"DecodeParms") {
            Some(Obj::Array(params)) => params.first().cloned(),
            other => other.cloned(),
        };
        let predictor = params
            .as_ref()
            .and_then(|p| p.get(b"Predictor"))
            .and_then(Obj::int);
        match predictor {
            None | Some(1) => Some(data),
            Some(10..=15) => {
                let param = |key: &[u8], default: i64| {
                    let value = params.as_ref().and_then(|p| p.get(key)).and_then(Obj::int);
                    usize::try_from(value.unwrap_or(default)).ok()
                };
                let (columns, colors, bits) = (
                    param(b"Columns", 1)?,
                    param(b"Colors", 1)?,
                    param(b"BitsPerComponent", 8)?,
                );
                unpredict(
                    &data,
                    columns.checked_mul(colors)?.checked_mul(bits)?.div_ceil(8),
                    (colors * bits).div_ceil(8),
                )
            }
            _ => None,
        }
    }
}

/// 去掉 PNG 预测：每行开头一个字节说明本行的预测方式，`row` 是每行的字节数，`bpp` 是一个像素
/// 的字节数（至少 1）。
fn unpredict(data: &[u8], row: usize, bpp: usize) -> Option<Vec<u8>> {
    if row == 0 {
        return None;
    }
    let bpp = bpp.max(1);
    let mut out = Vec::with_capacity(data.len());
    let mut previous = vec![0u8; row];
    for line in data.chunks(row + 1) {
        let (&kind, bytes) = line.split_first()?;
        let mut current = bytes.to_vec();
        current.resize(row, 0);
        for i in 0..row {
            let left = if i >= bpp { current[i - bpp] } else { 0 };
            let up = previous[i];
            let corner = if i >= bpp { previous[i - bpp] } else { 0 };
            let guess = match kind {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                4 => paeth(left, up, corner),
                _ => return None,
            };
            current[i] = current[i].wrapping_add(guess);
        }
        out.extend_from_slice(&current);
        previous = current;
    }
    Some(out)
}

/// PNG 的 Paeth 预测：取左、上、左上三者中最接近 `左 + 上 − 左上` 的一个。
fn paeth(left: u8, up: u8, corner: u8) -> u8 {
    let p = i16::from(left) + i16::from(up) - i16::from(corner);
    let (a, b, c) = (
        (p - i16::from(left)).abs(),
        (p - i16::from(up)).abs(),
        (p - i16::from(corner)).abs(),
    );
    if a <= b && a <= c {
        left
    } else if b <= c {
        up
    } else {
        corner
    }
}

/// `obj` 关键字（在 `at` 处）所属的 `n g obj` 的起点：往前跳过空白、代号、空白、对象号。
fn object_start(bytes: &[u8], at: usize) -> Option<usize> {
    let is_space = |b: u8| matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0');
    let back_over = |mut i: usize, digits: bool| {
        let start = i;
        while i > 0
            && (if digits {
                bytes[i - 1].is_ascii_digit()
            } else {
                is_space(bytes[i - 1])
            })
        {
            i -= 1;
        }
        (i < start).then_some(i)
    };
    // `obj` 后面必须是分隔符：不是 `objects` 之类的一部分。
    if bytes
        .get(at + 3)
        .is_some_and(|&b| !is_space(b) && !b"<[/(%".contains(&b))
    {
        return None;
    }
    let i = back_over(at, false)?;
    let i = back_over(i, true)?;
    let i = back_over(i, false)?;
    let i = back_over(i, true)?;
    (i == 0 || is_space(bytes[i - 1])).then_some(i)
}

/// 逐个读出对象的词法分析器。
struct Lexer<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(bytes: &'a [u8], pos: usize) -> Lexer<'a> {
        Lexer { bytes, pos }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// 跳过空白和注释（`%` 到行尾）。
    fn skip_space(&mut self) {
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0' => self.pos += 1,
                b'%' => {
                    while self.peek().is_some_and(|b| b != b'\n' && b != b'\r') {
                        self.pos += 1;
                    }
                }
                _ => break,
            }
        }
    }

    /// 当前位置是关键字 `word`（后面跟着分隔符）时越过它，返回 `true`。
    fn keyword(&mut self, word: &[u8]) -> bool {
        let rest = &self.bytes[self.pos.min(self.bytes.len())..];
        let ends = rest.get(word.len()).is_none_or(|&b| delimiter(b));
        if rest.starts_with(word) && ends {
            self.pos += word.len();
            true
        } else {
            false
        }
    }

    /// 读一个对象；`depth` 是数组、字典的嵌套层数。整数后面跟着 `g R` 时读成间接引用。
    fn object(&mut self, depth: usize) -> Option<Obj> {
        if depth > NEST_LIMIT {
            return None;
        }
        self.skip_space();
        let b = self.peek()?;
        match b {
            b'/' => {
                self.pos += 1;
                Some(Obj::Name(self.name()))
            }
            b'(' => {
                self.pos += 1;
                Some(Obj::Str(self.literal()))
            }
            b'<' if self.bytes.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                let mut dict = Vec::new();
                loop {
                    self.skip_space();
                    if self.bytes[self.pos.min(self.bytes.len())..].starts_with(b">>") {
                        self.pos += 2;
                        return Some(Obj::Dict(dict));
                    }
                    let Obj::Name(key) = self.object(depth + 1)? else {
                        return None;
                    };
                    let value = self.object(depth + 1)?;
                    dict.push((key, value));
                }
            }
            b'<' => {
                self.pos += 1;
                Some(Obj::Str(self.hex()))
            }
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_space();
                    if self.peek()? == b']' {
                        self.pos += 1;
                        return Some(Obj::Array(items));
                    }
                    items.push(self.object(depth + 1)?);
                }
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => self.number(),
            _ if self.keyword(b"true") => Some(Obj::Bool(true)),
            _ if self.keyword(b"false") => Some(Obj::Bool(false)),
            _ if self.keyword(b"null") => Some(Obj::Null),
            _ => None,
        }
    }

    /// 读一个数；是整数且后面跟着 `g R` 时读成间接引用。
    fn number(&mut self) -> Option<Obj> {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|b| matches!(b, b'+' | b'-' | b'.' | b'0'..=b'9'))
        {
            self.pos += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos]).ok()?;
        let Ok(n) = text.parse::<i64>() else {
            return text.parse::<f64>().ok().map(Obj::Real);
        };
        // 试着读 `代号 R`；不是时退回，整数照常返回。
        let back = self.pos;
        self.skip_space();
        let generation = self.peek().is_some_and(|b| b.is_ascii_digit()).then(|| {
            let from = self.pos;
            while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                self.pos += 1;
            }
            std::str::from_utf8(&self.bytes[from..self.pos])
                .ok()?
                .parse::<u16>()
                .ok()
        });
        if let Some(Some(generation)) = generation {
            self.skip_space();
            if self.keyword(b"R")
                && let Ok(n) = u32::try_from(n)
            {
                return Some(Obj::Ref(n, generation));
            }
        }
        self.pos = back;
        Some(Obj::Int(n))
    }

    /// 读名字（已越过 `/`），解开 `#xx`。
    fn name(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        while let Some(b) = self.peek().filter(|&b| !delimiter(b)) {
            self.pos += 1;
            if b == b'#'
                && let Some(v) = self
                    .bytes
                    .get(self.pos..self.pos + 2)
                    .and_then(|p| hex_byte(p.try_into().ok()?))
            {
                out.push(v);
                self.pos += 2;
                continue;
            }
            out.push(b);
        }
        out
    }

    /// 读字面字符串（已越过 `(`）：括号可以成对嵌套，解开反斜杠转义。
    fn literal(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut depth = 0usize;
        while let Some(b) = self.peek() {
            self.pos += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' if depth == 0 => break,
                b')' => {
                    depth -= 1;
                    out.push(b);
                }
                b'\\' => {
                    let Some(e) = self.peek() else { break };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        // 反斜杠加换行：续行，两者都不算内容。
                        b'\r' => {
                            if self.peek() == Some(b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut value = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(d @ b'0'..=b'7') => {
                                        value = value * 8 + u32::from(d - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(value as u8);
                        }
                        other => out.push(other),
                    }
                }
                other => out.push(other),
            }
        }
        out
    }

    /// 读十六进制字符串（已越过 `<`）：空白忽略，奇数个数字时末尾补 0。
    fn hex(&mut self) -> Vec<u8> {
        let mut digits = Vec::new();
        while let Some(b) = self.peek() {
            self.pos += 1;
            if b == b'>' {
                break;
            }
            if b.is_ascii_hexdigit() {
                digits.push(b);
            }
        }
        if digits.len() % 2 == 1 {
            digits.push(b'0');
        }
        (digits.as_chunks::<2>().0.iter())
            .filter_map(hex_byte)
            .collect()
    }
}

/// 两个十六进制数字表示的字节。
fn hex_byte(pair: &[u8; 2]) -> Option<u8> {
    let text = std::str::from_utf8(pair).ok()?;
    u8::from_str_radix(text, 16).ok()
}

/// `b` 是否是 PDF 的空白或分隔符。
fn delimiter(b: u8) -> bool {
    matches!(
        b,
        b' ' | b'\t'
            | b'\r'
            | b'\n'
            | b'\x0c'
            | b'\0'
            | b'('
            | b')'
            | b'<'
            | b'>'
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b'/'
            | b'%'
    )
}
