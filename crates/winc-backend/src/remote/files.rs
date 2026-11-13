//! 连接上的文件操作：元数据、读取、目录、写入、重命名、复制、删除和保存。

use super::*;

impl Connection {
    /// 取若干远端路径的文件信息（跟随符号链接），一次往返，结果与 `paths` 的顺序对应。
    pub(crate) fn metadata_many(&self, paths: &[&[u8]]) -> io::Result<Vec<io::Result<Metadata>>> {
        let requests: Vec<(u8, Vec<u8>)> = (paths.iter())
            .map(|path| (kind::METADATA, Writer::default().bytes(path).finish()))
            .collect();
        let decode = |reply: Vec<u8>| {
            let mut reply = Reader::new(&reply);
            let metadata = wire::read_metadata(&mut reply)?;
            reply.finish()?;
            Ok(metadata)
        };
        let replies = self.batch(&requests)?;
        Ok((replies.into_iter())
            .map(|reply| reply.and_then(decode))
            .collect())
    }

    /// 取远端文件 `path` 的文件信息，并读出其开头至多 `len` 字节，一次往返；返回文件信息、
    /// 数据和是否已到文件末尾。`path` 是目录等不能读取的对象时数据是相应的错误。
    pub(crate) fn open(&self, path: &[u8], len: u32) -> io::Result<(Metadata, io::Result<Chunk>)> {
        let read = Writer::default().bytes(path).u64(0).u32(len).finish();
        let requests = [
            (kind::METADATA, Writer::default().bytes(path).finish()),
            (kind::READ, read),
        ];
        let mut replies = self.batch(&requests)?.into_iter();
        let (Some(metadata), Some(data)) = (replies.next(), replies.next()) else {
            return Err(wire::invalid("short batch reply"));
        };
        let mut reply_metadata = metadata?;
        let metadata = {
            let mut reply = Reader::new(&reply_metadata);
            let metadata = wire::read_metadata(&mut reply)?;
            reply.finish()?;
            reply_metadata.clear();
            metadata
        };
        let data = data.and_then(|reply| {
            let mut reply = Reader::new(&reply);
            let data = reply.bytes()?.to_vec();
            let end = reply.u8()? != 0;
            reply.finish()?;
            if data.len() > len as usize {
                return Err(wire::invalid("read returned more than requested"));
            }
            Ok((data, end))
        });
        Ok((metadata, data))
    }

    /// 取远端路径 `path` 的文件信息；`follow` 为真时跟随符号链接。
    pub(crate) fn metadata(&self, path: &[u8], follow: bool) -> io::Result<Metadata> {
        let request = if follow {
            kind::METADATA
        } else {
            kind::SYMLINK_METADATA
        };
        let reply = self.request(request, &Writer::default().bytes(path).finish())?;
        let mut reply = Reader::new(&reply);
        let metadata = wire::read_metadata(&mut reply)?;
        reply.finish()?;
        Ok(metadata)
    }

    /// 列出远端目录 `path`，返回各项的名字（远端的原始字节）和种类。
    pub(crate) fn read_dir(&self, path: &[u8]) -> io::Result<Vec<(Vec<u8>, Kind)>> {
        let request = Writer::default().bytes(path).finish();
        let (_, replies) = self.link.send(kind::READ_DIR, &request)?;
        let mut entries = Vec::new();
        // 目录项分批到达，最后一帧是空的应答或错误。
        loop {
            let Ok(frame) = replies.recv() else {
                return Err(unavailable(Unavailable::Lost));
            };
            if frame.kind != kind::CHUNK {
                Reader::new(&finish(frame)?).finish()?;
                return Ok(entries);
            }
            let mut batch = Reader::new(&frame.payload);
            for _ in 0..batch.u32()? {
                let kind = wire::read_kind(batch.u8()?)?;
                let name = batch.bytes()?;
                // 名字来自对端：含 `/`、为空或是 `.`、`..` 的名字拼进路径后会指向别处，不接受。
                if name.is_empty() || name.contains(&b'/') || name == b"." || name == b".." {
                    return Err(wire::invalid("invalid entry name"));
                }
                entries.push((name.to_vec(), kind));
            }
            batch.finish()?;
        }
    }

    /// 读取远端文件 `path` 从 `offset` 起至多 `len` 字节（不超过 [`wire::MAX_READ`]），返回数据和
    /// 是否已到文件末尾。
    pub(crate) fn read_range(
        &self,
        path: &[u8],
        offset: u64,
        len: u32,
    ) -> io::Result<(Vec<u8>, bool)> {
        let request = Writer::default()
            .bytes(path)
            .u64(offset)
            .u32(len.min(wire::MAX_READ))
            .finish();
        let (_, replies) = self.link.send(kind::READ, &request)?;
        // 较长的数据分段到达：前面各段是 `CHUNK`，最后一帧带着余下的数据和是否已到末尾。
        let mut data = Vec::new();
        let end = loop {
            let Ok(frame) = replies.recv() else {
                return Err(unavailable(Unavailable::Lost));
            };
            let last = frame.kind != kind::CHUNK;
            let payload = if last { finish(frame)? } else { frame.payload };
            let mut reply = Reader::new(&payload);
            let part = if last { reply.bytes()? } else { &payload };
            // 调用方按请求的长度准备缓冲区：对端多给的数据不能接受。
            if data.len() + part.len() > len as usize {
                return Err(wire::invalid("read returned more than requested"));
            }
            data.extend_from_slice(part);
            if last {
                let end = reply.u8()? != 0;
                reply.finish()?;
                break end;
            }
        };
        Ok((data, end))
    }

    /// 读取远端文件 `path` 的全部内容，按 [`wire::MAX_READ`] 分块请求。
    pub(crate) fn read(&self, path: &[u8]) -> io::Result<Vec<u8>> {
        let mut content = Vec::new();
        loop {
            let (data, end) = self.read_range(path, content.len() as u64, wire::MAX_READ)?;
            // 未到末尾却没有数据：对端不守约定，继续请求不会结束。
            if data.is_empty() && !end {
                return Err(wire::invalid("empty read before the end"));
            }
            content.extend_from_slice(&data);
            if end {
                return Ok(content);
            }
        }
    }

    /// 读取远端符号链接 `path` 的目标，返回其原始字节。
    pub(crate) fn read_link(&self, path: &[u8]) -> io::Result<Vec<u8>> {
        let reply = self.request(kind::READ_LINK, &Writer::default().bytes(path).finish())?;
        let mut reply = Reader::new(&reply);
        let target = reply.bytes()?.to_vec();
        reply.finish()?;
        Ok(target)
    }

    /// 创建远端目录 `path`；`all` 为真时连同各级上级目录。
    pub(crate) fn create_dir(&self, path: &[u8], all: bool) -> io::Result<()> {
        let payload = Writer::default().bytes(path).flag(all).finish();
        self.command(kind::CREATE_DIR, payload)
    }

    /// 新建远端文件 `path` 并写入 `contents`，已有条目时不覆盖。
    pub(crate) fn create_new(&self, path: &[u8], contents: &[u8]) -> io::Result<()> {
        let payload = Writer::default().bytes(path).bytes(contents).finish();
        self.command(kind::CREATE_NEW, payload)
    }

    /// 把 `contents` 覆盖写入远端文件 `path`。
    pub(crate) fn write(&self, path: &[u8], contents: &[u8]) -> io::Result<()> {
        let payload = Writer::default().bytes(path).bytes(contents).finish();
        self.command(kind::WRITE, payload)
    }

    /// 把远端的 `from` 改名为 `to`。
    pub(crate) fn rename(&self, from: &[u8], to: &[u8]) -> io::Result<()> {
        self.command(
            kind::RENAME,
            Writer::default().bytes(from).bytes(to).finish(),
        )
    }

    /// 把远端文件 `from` 复制到 `to`，返回复制的字节数。
    pub(crate) fn copy(&self, from: &[u8], to: &[u8]) -> io::Result<u64> {
        let payload = Writer::default().bytes(from).bytes(to).finish();
        let reply = self.request(kind::COPY, &payload)?;
        let mut reply = Reader::new(&reply);
        let copied = reply.u64()?;
        reply.finish()?;
        Ok(copied)
    }

    /// 把远端的符号链接 `from` 复制为指向同一目标的链接 `to`。
    pub(crate) fn copy_symlink(&self, from: &[u8], to: &[u8]) -> io::Result<()> {
        let payload = Writer::default().bytes(from).bytes(to).finish();
        self.command(kind::COPY_SYMLINK, payload)
    }

    /// 把远端文件 `path` 的权限位设为 `mode`。
    pub(crate) fn set_mode(&self, path: &[u8], mode: u32) -> io::Result<()> {
        let payload = Writer::default().bytes(path).u32(mode).finish();
        self.command(kind::SET_MODE, payload)
    }

    /// 在远端创建指向 `target` 的符号链接 `link`。
    pub(crate) fn symlink(&self, target: &[u8], link: &[u8]) -> io::Result<()> {
        let payload = Writer::default().bytes(target).bytes(link).finish();
        self.command(kind::SYMLINK, payload)
    }

    /// 删除远端文件 `path`。
    pub(crate) fn remove_file(&self, path: &[u8]) -> io::Result<()> {
        self.command(kind::REMOVE_FILE, Writer::default().bytes(path).finish())
    }

    /// 删除远端的 `path`：目录连同其中的全部内容。
    pub(crate) fn remove_all(&self, path: &[u8]) -> io::Result<()> {
        self.command(kind::REMOVE_ALL, Writer::default().bytes(path).finish())
    }

    /// 把 `write` 写出的内容保存到远端文件 `path`，返回写入后的文件信息。
    ///
    /// 内容边写边分段发出，对端收齐后按 [`fs::save`](crate::fs::save) 的流程写入。`write` 失败时
    /// 通知对端丢弃已收到的内容，目标文件不变。
    pub(crate) fn save(
        &self,
        path: &[u8],
        bom: bool,
        in_memory: bool,
        write: impl FnOnce(&mut dyn Write) -> io::Result<()>,
    ) -> Result<Metadata, SaveError> {
        let begin = Writer::default()
            .bytes(path)
            .flag(bom)
            .flag(in_memory)
            .finish();
        let (id, replies) = self.link.send(kind::SAVE_BEGIN, &begin)?;
        let mut upload = Upload {
            link: &self.link,
            id,
            pending: Vec::new(),
        };
        let written = write(&mut upload).and_then(|()| upload.flush());
        let end = Writer::default().flag(written.is_ok()).finish();
        self.link.write(kind::SAVE_END, id, &end)?;
        let reply = match replies.recv() {
            Ok(frame) => finish(frame),
            Err(_) => Err(unavailable(Unavailable::Lost)),
        };
        // 本机一侧的错误（例如内容无法按所选的编码写出）优先于对端对「放弃」的应答。
        written?;
        let reply = reply?;
        let mut reply = Reader::new(&reply);
        match reply.u8()? {
            kind::SAVED => {
                let metadata = wire::read_metadata(&mut reply)?;
                reply.finish()?;
                Ok(metadata)
            }
            kind::SAVE_READ_ONLY => Err(SaveError::ReadOnly),
            kind::SAVE_IDENTITY => Err(SaveError::IdentityWouldChange),
            kind::SAVE_TEMPORARY => Err(SaveError::TemporaryFile),
            _ => Err(wire::invalid("unknown save result").into()),
        }
    }
}
