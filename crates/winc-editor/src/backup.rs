//! 未保存内容的备份：退出时无需逐个询问是否保存，下次启动时原样恢复；进程异常中止后也能找回。
//!
//! 每个目录（没有打开目录时为「空窗口」）的备份放在一个子目录里，每个文档一个文件：第一行是
//! `MAGIC` 前缀加一行 JSON 头（对应的文件、磁盘文件的修改时间、选区、标签序号、保存用的编码和
//! 行尾），其后是文档的全部内容（UTF-8）。同一目录被两个 winc 实例打开时，只有先启动的实例读取、
//! 写入、删除该目录的备份，即同一时刻只允许一个实例处理同一目录（[`Backups::lock`]）。

#![allow(
    clippy::disallowed_methods,
    reason = "只访问本机：备份保存在本机的数据目录中"
)]

use std::fs;
use std::io::{self, BufRead, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use winc_backend::fs::Stamp;

use ropey::Rope;
use winc_config::Json;
use winc_core::Selection;
use winc_core::text::LineEnding;

use crate::{Document, Editor, Encoding};

/// 备份文件第一行的前缀，用于识别文件是否由 winc 写出，以及格式版本（当前为 1）。
const MAGIC: &str = "winc-backup 1 ";
/// 第一行（[`MAGIC`] 加 JSON 头）最多读多少字节（64 KiB）：头里只有路径和几个数，读到该长度
/// 还没有换行的不是备份，不必把整个文件读进来。
const HEADER_LIMIT: u64 = 64 << 10;

/// 一个目录对应的备份集合，即存放这些备份的子目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backups {
    dir: PathBuf,
}

/// 对一个目录的备份的独占锁。
///
/// 由 [`Backups::lock`] 或 [`Backups::lock_within`] 取得。`Lock` 被丢弃、进程退出或异常
/// 中止时，系统自动释放该锁。
#[derive(Debug)]
pub struct Lock {
    /// 持有锁的文件句柄；关闭它即释放锁。
    _file: fs::File,
    /// 锁文件的路径，记在 [`HELD`] 中。
    path: PathBuf,
}

impl Drop for Lock {
    fn drop(&mut self) {
        if let Ok(mut held) = HELD.lock() {
            held.retain(|path| *path != self.path);
        }
    }
}

/// 本进程当前持有的各把锁的锁文件路径。
///
/// 取不到锁时据此分辨占用者：锁文件中记着本进程的编号、而这里没有该路径时，占用者是本进程
/// 启动的外部程序（见 [`Backups::lock_within`]）。
static HELD: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// 待写出的备份：记录取快照那一刻的内容（rope 以共享方式持有，不复制），可以交给其他线程写出。
pub struct Snapshot {
    /// 备份文件的完整路径。
    pub file: PathBuf,
    /// 第一行的 JSON 头（不含 [`MAGIC`] 前缀和换行符）。
    header: String,
    /// 取快照时的文档内容。
    rope: Rope,
}

/// 从磁盘读回的备份文件的头；正文要用时才读（[`Backup::text`]）。
///
/// 备份不限大小：启动时把所有备份的正文一起读进来，几份大的未保存内容加起来就是启动时的内存
/// 峰值。恢复时逐个读取，每恢复一个就释放它的正文，峰值只是最大的备份占用的内存。
#[derive(Debug, Clone, PartialEq)]
pub struct Backup {
    /// 备份文件的完整路径。
    pub file: PathBuf,
    /// 对应的文件；未命名的新文件为 `None`。
    pub path: Option<PathBuf>,
    /// 正文在文件里从第几个字节开始（第一行之后）。
    body: u64,
    /// 写备份时磁盘文件的修改时间。
    pub mtime: Option<Stamp>,
    /// 备份时的选区，恢复后光标回到原处。
    pub selection: Selection,
    /// 备份时的标签序号（从 0 起）；备份头中缺失时为 `usize::MAX`。
    pub tab: usize,
    /// 备份时文档是否处于冲突中（磁盘文件已被其他程序修改，用户还没选择保留哪一边）。
    ///
    /// 冲突时记下的修改时间已是对方写入后的时间，单靠修改时间看不出冲突，因此另外记下：否则
    /// 退出或异常中止后再打开，冲突就不见了，保存会覆盖对方的修改而不提示用户。
    pub conflict: bool,
    /// 备份时保存用的编码；备份头中缺失或不认识时为 `None`，恢复时沿用打开文件时识别出的编码。
    ///
    /// 正文一律按 UTF-8 存放，编码只在保存时用到。不记下时，磁盘上的文件已被删除（恢复成只带
    /// 路径的标签）、或用户按别的编码重新打开过的文件，恢复后保存会改用 UTF-8 或识别出的编码。
    pub encoding: Option<Encoding>,
    /// 备份时新输入的换行使用的行尾；缺失时为 `None`，沿用打开文件时识别出的行尾。
    pub line_ending: Option<LineEnding>,
}

impl Backups {
    /// 创建 `folder` 目录对应的备份集合，备份放在 `root` 下；`folder` 为 `None` 表示没有
    /// 打开目录的窗口。
    ///
    /// 子目录名是 `folder` 路径的 FNV-1a 哈希（16 位十六进制），空窗口使用 `empty`。
    pub fn new(root: &Path, folder: Option<&Path>) -> Backups {
        let key = folder.map_or_else(|| "empty".to_owned(), folder_key);
        Backups {
            dir: root.join(key),
        }
    }

    /// 返回备份所在的目录。
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 独占该目录的备份；另一个 winc 实例已经占用，或无法创建锁文件时返回 `None`。
    ///
    /// 后启动的实例也读取、删除备份时，会把先启动的实例仍在使用的备份当作上次遗留的内容恢复、保存后
    /// 又删除，先启动的实例之后再异常中止时内容就无法找回，因此同一时刻只允许一个实例处理同一目录。
    /// 加锁对象是目录旁边的 `<目录>.lock` 文件，使用建议锁且不删除该文件：删除后再创建时，两个进程
    /// 可能各锁一个文件。文件系统不支持建议锁时视为加锁成功。取到锁后把本进程的编号写入锁文件，
    /// 供 [`Self::lock_within`] 分辨占用者；写入失败不影响加锁。
    pub fn lock(&self) -> Option<Lock> {
        let path = self.lock_path();
        if let Some(root) = path.parent() {
            fs::create_dir_all(root).ok()?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .ok()?;
        match file.try_lock() {
            Err(fs::TryLockError::WouldBlock) => None,
            Ok(()) | Err(fs::TryLockError::Error(_)) => {
                let _ = file.set_len(0);
                let _ = file.write_all(std::process::id().to_string().as_bytes());
                if let Ok(mut held) = HELD.lock() {
                    held.push(path.clone());
                }
                Some(Lock { _file: file, path })
            }
        }
    }

    /// 同 [`Self::lock`]，占用者是本进程启动的外部程序时每隔 5 ms 重试，最多等待 `wait`。
    ///
    /// 本进程启动外部程序时，子进程在 fork 之后、exec 之前继承锁文件的描述符；本进程此时放开
    /// 锁，建议锁仍由子进程占用到 exec 为止。这一占用很快结束，重试到取到锁；占用者是另一个
    /// winc 实例时不等待，立即返回 `None`。
    pub fn lock_within(&self, wait: std::time::Duration) -> Option<Lock> {
        let deadline = std::time::Instant::now() + wait;
        loop {
            if let Some(lock) = self.lock() {
                return Some(lock);
            }
            if !self.held_by_own_child() || std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// 返回锁文件的路径。
    fn lock_path(&self) -> PathBuf {
        self.dir.with_extension("lock")
    }

    /// 锁的占用者是否为本进程启动的外部程序：锁文件中记着本进程的编号，而本进程没有持有它。
    ///
    /// 只有 Unix 上子进程会继承建议锁，其他平台总是返回 `false`。
    fn held_by_own_child(&self) -> bool {
        if !cfg!(unix) {
            return false;
        }
        let path = self.lock_path();
        let ours =
            fs::read_to_string(&path).is_ok_and(|pid| pid.trim().parse() == Ok(std::process::id()));
        ours && HELD.lock().is_ok_and(|held| !held.contains(&path))
    }

    /// 返回 `doc` 的备份文件路径。
    ///
    /// 有路径的文档按路径的哈希命名，路径变化（另存为、改名）后对应另一份备份；未命名的
    /// 文档使用 `untitled` 给出的名字，通常来自 [`untitled_name`]。
    pub fn file_for(&self, doc: &Document, untitled: &str) -> PathBuf {
        match &doc.path {
            Some(path) => self.dir.join(format!("{:016x}", hash(path))),
            None => self.dir.join(untitled),
        }
    }

    /// 是否需要备份 `doc`：有未保存的修改、可编辑、不是输出标签，且内容整份在内存里（不是按需
    /// 从磁盘读的大文件 `LargeText`：它的原文在磁盘上，退出时仍然询问是否保存）。
    ///
    /// 不限大小：大的文档也整份备份，只是备份间隔更长（winc-app 的 `backup_delay`）；写入时按块
    /// 流式写出，不另外复制整个文档。
    ///
    /// 正在预览的 SVG 仍然备份：预览只是暂时只读，内容仍然是用户未保存的修改。
    pub fn wanted(doc: &Document) -> bool {
        doc.is_dirty() && doc.text_editable() && doc.title.is_none() && doc.text().rope().is_some()
    }

    /// 取 `doc` 当前内容的快照；`tab` 是文档当前的标签序号（从 0 起）。
    ///
    /// 不需要备份时返回 `None`，条件见 [`Backups::wanted`]。
    pub fn snapshot(&self, doc: &Document, untitled: &str, tab: usize) -> Option<Snapshot> {
        if !Backups::wanted(doc) {
            return None;
        }
        let rope = doc.text().rope()?.clone();

        let mut fields = vec![
            ("anchor".to_owned(), number(doc.selection.anchor)),
            ("head".to_owned(), number(doc.selection.head)),
            ("tab".to_owned(), number(tab)),
        ];
        if let Some(path) = &doc.path {
            let path = path.to_string_lossy().into_owned();
            fields.push(("path".to_owned(), Json::String(path)));
        }
        // 纪元之前的修改时间不记录：读取时只接受非负的秒数。
        if let Some(stamp) = doc.disk_mtime().filter(|stamp| stamp.secs >= 0) {
            let mtime = vec![
                Json::Number(stamp.secs as f64),
                Json::Number(f64::from(stamp.nanos)),
            ];
            fields.push(("mtime".to_owned(), Json::Array(mtime)));
        }
        if doc.in_conflict() {
            fields.push(("conflict".to_owned(), Json::Bool(true)));
        }
        if let Some(key) = doc.encoding.key() {
            fields.push(("encoding".to_owned(), Json::String(key.to_owned())));
        }
        let eol = match doc.line_ending {
            LineEnding::Lf => "lf",
            LineEnding::CrLf => "crlf",
            LineEnding::Cr => "cr",
        };
        fields.push(("eol".to_owned(), Json::String(eol.to_owned())));
        let header = Json::Object(fields.into_iter().collect()).compact();

        Some(Snapshot {
            file: self.file_for(doc, untitled),
            header,
            rope,
        })
    }

    /// 读出该目录中的全部备份（只读头，正文见 [`Backup::text`]），按备份时的标签序号排序
    /// （序号相同时按文件路径）。
    ///
    /// 无法读取或头的格式不对的文件被跳过，遗留的 `.tmp` 临时文件会被删除。调用前必须已独占
    /// 该目录（[`Backups::lock`]）。
    pub fn read_all(&self) -> Vec<Backup> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut backups: Vec<Backup> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            // 写到一半断电留下的临时文件不是备份：其内容可能不完整，恢复它会覆盖完好的备份。
            // 该目录已被本实例独占，没有其他写入者，遗留的临时文件可以全部删除。
            .filter(|e| {
                let tmp = e.path().extension().is_some_and(|x| x == "tmp");
                if tmp {
                    let _ = fs::remove_file(e.path());
                }
                !tmp
            })
            .filter_map(|e| read_header(&e.path()))
            .collect();
        backups.sort_by(|a, b| a.tab.cmp(&b.tab).then_with(|| a.file.cmp(&b.file)));
        backups
    }
}

impl Backup {
    /// 读出备份的正文。
    ///
    /// # Errors
    ///
    /// 文件读不了，或正文不是 UTF-8 文本时返回对应的 `std::io::Error`。
    pub fn text(&self) -> io::Result<String> {
        let mut file = fs::File::open(&self.file)?;
        let len = file.metadata()?.len().saturating_sub(self.body);
        file.seek(io::SeekFrom::Start(self.body))?;
        let mut text = String::with_capacity(usize::try_from(len).unwrap_or(0));
        file.read_to_string(&mut text)?;
        Ok(text)
    }
}

impl Snapshot {
    /// 第一行的 JSON 头：光标位置、标签序号、对应的文件等，不含正文。
    pub fn header(&self) -> &str {
        &self.header
    }

    /// 写入磁盘：先在同一目录下新建临时文件并写入（不覆盖已有文件），同步到磁盘后再改名覆盖
    /// 目标文件。
    ///
    /// 写入过程中断电不会损坏原有的备份。
    ///
    /// # Errors
    ///
    /// 创建目录、写临时文件、同步或改名失败时返回对应的 `std::io::Error`。
    pub fn write(&self) -> io::Result<()> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);

        let dir = self.file.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(dir)?;
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let tmp = self
            .file
            .with_extension(format!("{:x}-{n:x}.tmp", std::process::id()));

        let result = (|| {
            let file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            let mut out = io::BufWriter::with_capacity(1 << 20, file);
            out.write_all(MAGIC.as_bytes())?;
            out.write_all(self.header.as_bytes())?;
            out.write_all(b"\n")?;
            for chunk in self.rope.chunks() {
                out.write_all(chunk.as_bytes())?;
            }
            let file = out.into_inner().map_err(io::IntoInnerError::into_error)?;
            // 只保证内容先于改名落盘（`winc_config::files::sync_file`），不等磁盘清空写缓存：
            // 备份在编辑的间隙反复写，断电时最多丢失最近这一次。
            winc_config::files::sync_file(&file)?;
            winc_config::files::replace_file(&tmp, &self.file)?;
            crate::document::sync_dir(dir);
            Ok(())
        })();

        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }
}

/// 删除备份文件（文档已保存、已放弃或已关闭），所在目录变空时一并删除目录。
///
/// 文件本来就不存在时同样返回成功。
///
/// # Errors
///
/// 删除文件或目录失败时返回对应的 `std::io::Error`。
pub fn remove(file: &Path) -> io::Result<()> {
    match fs::remove_file(file) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    if let Some(dir) = file.parent() {
        // 目录内还有其他备份时 `remove_dir` 会失败，这里需要的正是该行为，因此忽略错误。
        let _ = fs::remove_dir(dir);
    }
    Ok(())
}

/// 为未命名的文档生成备份文件名。
///
/// 名字由时间（纳秒）、进程号和计数组成，同一目录中不会与已有的备份重名。
pub fn untitled_name() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("untitled-{now:x}-{:x}-{n:x}", std::process::id())
}

impl Editor {
    /// 把一个备份恢复为标签，返回（标签序号，是否冲突）。
    ///
    /// 对应的文件已打开时恢复到该标签；没有打开但磁盘上存在时先打开它；磁盘上已不存在时创建一个
    /// 只带路径的标签；未命名的备份新建一个标签。冲突指备份之后磁盘上的文件被其他程序修改过。
    ///
    /// 文件现在无法打开或只读（例如被替换成二进制文件）时，内容放进一个新的未命名标签，不会丢失；
    /// 备份的正文读不出来（文件损坏、不是 UTF-8 文本）时不恢复，返回 `None`，备份文件保留不变。
    pub fn restore_backup(&mut self, backup: &Backup) -> Option<(usize, bool)> {
        // 正文在这里才读，读完这一个备份就释放，不和其他备份的正文同时留在内存里。
        let text = backup.text().ok()?;
        let index = match &backup.path {
            Some(path) if path.exists() => Some(self.open(path)),
            Some(path) => {
                let id = self.allocate_id();
                self.documents
                    .push(Document::missing(id, path.clone(), backup.mtime));
                Some(self.documents.len() - 1)
            }
            None => None,
        };

        if let Some(index) = index {
            let doc = &mut self.documents[index];
            if let Some(conflict) =
                doc.restore_unsaved(&text, backup.mtime, backup.selection, backup.conflict)
            {
                restore_format(doc, backup);
                return Some((index, conflict));
            }
        }

        let index = self.new_untitled();
        let doc = &mut self.documents[index];
        doc.restore_unsaved(&text, None, backup.selection, false);
        restore_format(doc, backup);
        Some((index, false))
    }
}

/// 把备份记下的编码和行尾设回 `doc`。
///
/// 文档不能按记下的编码保存（大文件只能用 UTF-8）时保留文档自己的编码。
fn restore_format(doc: &mut Document, backup: &Backup) {
    if let Some(encoding) = backup.encoding {
        doc.set_save_encoding(encoding);
    }
    if let Some(line_ending) = backup.line_ending {
        doc.line_ending = line_ending;
    }
}

/// 读取备份文件的头（第一行）；无法读取或头的格式不对时返回 `None`。
fn read_header(file: &Path) -> Option<Backup> {
    let mut first = Vec::new();
    io::BufReader::new(fs::File::open(file).ok()?.take(HEADER_LIMIT))
        .read_until(b'\n', &mut first)
        .ok()?;
    let line = std::str::from_utf8(first.strip_suffix(b"\n")?).ok()?;
    let header = Json::parse(line.strip_prefix(MAGIC)?)?;
    let header = header.as_object()?;

    let count = |key: &str| -> Option<usize> {
        header
            .get(key)?
            .as_number()
            .filter(|n| *n >= 0.0 && n.fract() == 0.0)
            .map(|n| n as usize)
    };
    let mtime = header.get("mtime").and_then(Json::as_array).and_then(|t| {
        let secs = t.first()?.as_number()?;
        let nanos = t.get(1)?.as_number()?;
        // 损坏的头（数值过大、为负或 NaN）不能引发 panic：备份在每次启动时读取，panic 会使
        // 程序一直无法启动。
        if !(0.0..=i64::MAX as f64).contains(&secs) || !(0.0..1e9).contains(&nanos) {
            return None;
        }
        Some(Stamp {
            secs: secs as i64,
            nanos: nanos as u32,
        })
    });

    Some(Backup {
        file: file.to_path_buf(),
        path: header
            .get("path")
            .and_then(Json::as_str)
            .filter(|p| !p.is_empty())
            .map(PathBuf::from),
        body: first.len() as u64,
        mtime,
        selection: Selection::new(count("anchor")?, count("head")?),
        tab: count("tab").unwrap_or(usize::MAX),
        conflict: header
            .get("conflict")
            .and_then(Json::as_bool)
            .unwrap_or(false),
        encoding: header
            .get("encoding")
            .and_then(Json::as_str)
            .and_then(Encoding::from_key),
        line_ending: match header.get("eol").and_then(Json::as_str) {
            Some("lf") => Some(LineEnding::Lf),
            Some("crlf") => Some(LineEnding::CrLf),
            Some("cr") => Some(LineEnding::Cr),
            _ => None,
        },
    })
}

fn number(n: usize) -> Json {
    Json::Number(n as f64)
}

/// 返回目录 `folder` 的短名字（路径的 FNV-1a 哈希，16 位十六进制）：备份子目录以它命名，
/// 同一目录在各个 winc 进程中算出的名字相同。
pub fn folder_key(folder: &Path) -> String {
    format!("{:016x}", hash(folder))
}

/// 返回路径字节的 64 位 FNV-1a 哈希（[`winc_core::hash`]）。
///
/// 结果在每次运行、每台机器上都相同；`std` 的哈希器每次运行使用不同的种子，不能用于这里。
fn hash(path: &Path) -> u64 {
    winc_core::hash::fnv(path.as_os_str().as_encoded_bytes().iter().copied())
}

#[cfg(test)]
mod tests;
