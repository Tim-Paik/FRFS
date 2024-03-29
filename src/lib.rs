//! # FRFS: File-based Read-only File System
//!
//! FRFS allows you to pack tons of small files into a single file,
//! and provides the ability to read randomly, avoid excessively
//! long paths, and improve copy efficiency.
//! Recommended for small file packaging.
//!
//! Most APIs are similar to std::fs, but read only.
//!
//! The difference from `std::fs` is that the ReadDir returned by
//! `read_dir` has the `into_iter()` method instead of `iter()`.
//!
//! ## Main Function
//!
//! [load]: Load a FRFS file.
//!
//! [load_from_file]: Load a FRFS file from frfs::File.
//!
//! [pack]: Pack a folder into a FRFS file.
//!
//! ## Example
//!
//! ```rust
//! fn main() -> std::io::Result<()> {
//!     // Pack a frfs file
//!     frfs::pack("src", "./src.frfs")?;
//!     // then open it
//!     let fs = frfs::load("./src.frfs")?;
//!     // iterating over and printing what's inside is as easy as using std::fs
//!     for entry in fs.read_dir("/")? {
//!         let entry = entry?;
//!         if entry.metadata()?.is_file() {
//!             println!("{:?}", fs.open(entry.path())?);
//!         } else if entry.metadata()?.is_dir() {
//!             println!("Dir {{ path: {:?} }}", entry.path());
//!         }
//!     }
//!     Ok(())
//! }
//! ```
//!

mod read_at;

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fmt::Debug;
use std::hash::Hash;
use std::io::{self, ErrorKind, Read, Result, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::{fs, path};

use bincode::Options;
use serde::{Deserialize, Serialize};

// 文件开头和结尾的 Magic Number, 用于检查文件完整性
pub type MagicNumber = [u8; 7];
const MAGIC_NUMBER_START: MagicNumber = *b"FRFSv02";
const MAGIC_NUMBER_END: MagicNumber = *b"FRFSEnd";
const U64_LEN: usize = std::mem::size_of::<u64>();

/// Internal error type
#[non_exhaustive]
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("IO Error: {0}")]
    IO(io::Error),
    #[error("An entity was not found, often a file or a dir")]
    NotFound,
    #[error("Illegal data: not a FRFS file")]
    IllegalData,
    #[error("Serialization error: {0}")]
    SerializationError(String),
    #[error("Deserialization error: {0}")]
    DeserializationError(String),
    #[error("Unknown error: {0}")]
    Unknown(String),
}

impl From<io::Error> for Error {
    fn from(io_error: io::Error) -> Self {
        Self::IO(io_error)
    }
}

impl From<Error> for io::Error {
    fn from(error: Error) -> Self {
        match error {
            Error::IO(io_error) => io_error,
            Error::NotFound => io::Error::new(ErrorKind::NotFound, error.to_string()),
            Error::IllegalData => io::Error::new(ErrorKind::InvalidData, error.to_string()),
            Error::SerializationError(_) => {
                io::Error::new(ErrorKind::InvalidData, error.to_string())
            }
            Error::DeserializationError(_) => {
                io::Error::new(ErrorKind::InvalidData, error.to_string())
            }
            Error::Unknown(_) => io::Error::new(ErrorKind::InvalidData, error.to_string()),
        }
    }
}

/// The header of a file.
/// It is stored in the [`Dir`] at the start of a FRFS file.
/// It contains the basic file infomation.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct FileHeader {
    /// The size of the file.
    pub file_size: u64,
    /// The offset in the [`File::source`].
    pub start_at: u64,
}

/// The file type returned by the open function implements most of the APIs of std::fs::File
#[derive(Debug)]
pub struct File {
    /// The information of the file.
    header: FileHeader,
    /// The offset (file cursor) of the file.
    /// We use the same source in FRFS.
    /// Therefore we cannot use the file cursor inside the file handle.
    ///
    /// It should not larger than header.file_size.
    offset: u64,
    /// The source of the file.
    /// It should be a FRFS file.
    source: fs::File,
}

impl File {
    pub fn open(fs: &FRFS, path: impl AsRef<Path>) -> Result<File> {
        fs.open(path)
    }

    pub fn metadata(&self) -> Result<Metadata> {
        Ok(Metadata(self.header.file_size, FileType(true)))
    }
}

impl From<fs::File> for File {
    fn from(mut f: fs::File) -> Self {
        Self {
            header: FileHeader {
                file_size: f.metadata().unwrap().len(),
                start_at: 0,
            },
            // The input fs::File may not at the start.
            offset: f.stream_position().unwrap(),
            source: f,
        }
    }
}

impl Read for File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        // Calculate remain file size.
        // This is the largest size the user can read.
        let remain_size = (self.header.file_size - self.offset) as usize;
        // Slice the buffer to make sure no overflow.
        let buf = if buf.len() > remain_size {
            &mut buf[..remain_size]
        } else {
            buf
        };
        // The real offset should add self.header.start_at
        let ret = read_at::read_at(&self.source, buf, self.offset + self.header.start_at)?;
        // ...and update the offset.
        self.offset += ret as u64;
        Ok(ret)
    }
}

impl Seek for File {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        match pos {
            // Current: simply add the offset.
            // But it should not larger than file_size.
            SeekFrom::Current(offset) => {
                self.offset = self
                    .offset
                    .wrapping_add_signed(offset)
                    .min(self.header.file_size);
            }
            // Start: simply assign.
            // But it should not larger than file_size.
            SeekFrom::Start(offset) => {
                self.offset = offset.min(self.header.file_size);
            }
            // End: sub from file_size.
            // It cannot be larger than file_size.
            SeekFrom::End(offset) => {
                self.offset = if offset >= 0 {
                    self.header.file_size
                } else {
                    self.header.file_size.wrapping_add_signed(offset)
                }
            }
        }
        Ok(self.offset)
    }
}

/// Metadata information about a file.
/// This structure is returned from the metadata function or method and
/// represents known metadata about a file such as its permissions, size, etc.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Metadata(u64, FileType);
#[derive(Clone, PartialEq, Eq, Debug)]
/// Representation of the various permissions on a file.
pub struct Permissions;
/// A structure representing a type of file with accessors for each file type.
/// It is returned by [Metadata::file_type] method.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct FileType(bool);
/// Iterator over the entries in a directory.
/// This iterator is returned from the [read_dir](FRFS::read_dir) function of this module.
#[derive(Debug)]
pub struct ReadDir {
    data: Vec<Result<DirEntry>>,
}
/// Entries returned by the [ReadDir] iterator.
#[derive(Clone, Debug)]
pub struct DirEntry {
    path: PathBuf,
    is_file: bool,
    file_size: u64,
}

/// read only, create/access/modify time returns UNIX_EPOCH
impl Metadata {
    pub fn file_type(&self) -> FileType {
        self.1
    }

    pub fn is_dir(&self) -> bool {
        self.file_type().is_dir()
    }

    pub fn is_file(&self) -> bool {
        self.file_type().is_file()
    }

    pub fn is_symlink(&self) -> bool {
        self.file_type().is_symlink()
    }

    pub fn len(&self) -> u64 {
        self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }

    pub fn permissions(&self) -> Permissions {
        Permissions
    }

    pub fn modified(&self) -> Result<std::time::SystemTime> {
        Ok(std::time::SystemTime::UNIX_EPOCH)
    }

    pub fn accessed(&self) -> Result<std::time::SystemTime> {
        Ok(std::time::SystemTime::UNIX_EPOCH)
    }

    pub fn created(&self) -> Result<std::time::SystemTime> {
        Ok(std::time::SystemTime::UNIX_EPOCH)
    }
}

/// Read Only
impl Permissions {
    pub fn readonly(&self) -> bool {
        true
    }
}

impl FileType {
    pub fn is_dir(&self) -> bool {
        !self.0
    }

    pub fn is_file(&self) -> bool {
        self.0
    }

    pub fn is_symlink(&self) -> bool {
        false
    }
}

impl IntoIterator for ReadDir {
    type Item = Result<DirEntry>;
    type IntoIter = std::vec::IntoIter<Self::Item>;
    fn into_iter(self) -> Self::IntoIter {
        self.data.into_iter()
    }
}

impl DirEntry {
    /// Returns the full path to the file that this entry represents.
    /// The full path is created by joining the original path to
    /// read_dir with the filename of this entry.
    /// Always full path relative to root.
    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }

    pub fn file_name(&self) -> OsString {
        self.path
            .file_name()
            .unwrap_or_else(|| OsStr::new(".."))
            .to_os_string()
    }

    pub fn metadata(&self) -> Result<Metadata> {
        Ok(Metadata(self.file_size, self.file_type()?))
    }

    pub fn file_type(&self) -> Result<FileType> {
        Ok(FileType(self.is_file))
    }
}

// Dir: 内建文件夹类型
// files: 此目录下文件
// dirs: 此目录下文件
#[derive(Serialize, Deserialize, Debug)]
struct Dir {
    files: HashMap<String, FileHeader>,
    dirs: HashMap<String, Dir>,
}

impl Dir {
    // 使用本地文件系统填充 Dir 结构体
    fn fill_with(&mut self, root: impl AsRef<Path>, path: impl AsRef<Path>) -> Result<u64> {
        let mut length = 0;
        // 遍历目录
        for entry in fs::read_dir(path)? {
            let path = entry?.path();
            // 获取目录/文件名，如果为".."则跳过
            let name = match path.file_name() {
                Some(s) => match s.to_str() {
                    Some(s) => s.to_string(),
                    None => break,
                },
                None => break,
            };
            // 优先填充文件
            if path.is_file() {
                let header = FileHeader {
                    file_size: std::fs::metadata(path)?.len(),
                    start_at: 0,
                };
                length += header.file_size;
                self.files.insert(name, header);
            } else if path.is_dir() {
                // 构造子目录
                let mut dir = Dir {
                    files: HashMap::new(),
                    dirs: HashMap::new(),
                };
                // 填充子目录
                length += dir.fill_with(root.as_ref(), &path)?;
                self.dirs.insert(name, dir);
            }
        }
        Ok(length)
    }
}

/// FRFS header.
// root: 根目录'/', 用于存储文件所在位置
// data_size: 在 base 文件中的 start_at 后所跟的数据大小
// start_at: 在 base 文件中的开始点，无需序列化
#[derive(Debug, Serialize, Deserialize)]
struct FRFSHeader {
    pub root: Dir,
    pub data_size: u64,
    pub start_at: u64,
}

// FRFS: 只读文件系统类型
// base: 文件源，无需序列化
/// Read-only File System
#[derive(Debug)]
pub struct FRFS {
    header: FRFSHeader,
    base: fs::File,
}

impl FRFS {
    pub fn new(p: impl AsRef<Path>) -> Result<Self> {
        Self::from_std_file(fs::File::open(p)?)
    }

    pub fn new_with_header(
        p: impl AsRef<Path>,
        magic_number_start: MagicNumber,
        magic_number_end: MagicNumber,
    ) -> Result<Self> {
        Self::from_std_file_with_header(fs::File::open(p)?, magic_number_start, magic_number_end)
    }

    /// Read FRFS from File.
    pub fn from_std_file(file: fs::File) -> Result<Self> {
        Self::from_file(file.into())
    }

    /// Read FRFS from File with header.
    pub fn from_std_file_with_header(
        file: fs::File,
        magic_number_start: MagicNumber,
        magic_number_end: MagicNumber,
    ) -> Result<Self> {
        Self::from_file_with_header(file.into(), magic_number_start, magic_number_end)
    }

    /// Load a FRFS file.
    pub fn from_file(base: File) -> Result<Self> {
        Self::from_file_with_header(base, MAGIC_NUMBER_START, MAGIC_NUMBER_END)
    }

    /// Load a FRFS file with header.
    pub fn from_file_with_header(
        mut base: File,
        magic_number_start: MagicNumber,
        magic_number_end: MagicNumber,
    ) -> Result<Self> {
        let mut magic_number_start_data = [0; MAGIC_NUMBER_START.len()];
        let mut header_length_data = [0; U64_LEN];
        let mut header_data = Vec::new();
        let mut data_length_data = [0; U64_LEN]; // 即创建文件时的 target_length 的 be_bytes 形式
        let mut magic_number_end_data = [0; MAGIC_NUMBER_END.len()];
        base.seek(SeekFrom::End(-(magic_number_end.len() as i64)))?;
        // 此时指针指向 magic_number_end 之前
        base.read_exact(&mut magic_number_end_data)?;
        if magic_number_end_data != magic_number_end {
            return Err(Error::IllegalData.into());
        }
        base.seek(SeekFrom::End(-((magic_number_end.len() + U64_LEN) as i64)))?;
        // 此时指针指向 data_length 之前
        base.read_exact(&mut data_length_data)?;
        base.seek(SeekFrom::End(
            -(u64::from_be_bytes(data_length_data) as i64),
        ))?;
        // 此时指针指向 magic_number_start
        base.read_exact(&mut magic_number_start_data)?;
        if magic_number_start_data != magic_number_start {
            return Err(Error::IllegalData.into());
        }
        base.read_exact(&mut header_length_data)?;
        // start_at 指 Data 段在 base 的位置
        let start_at = base.stream_position()?;
        let start_at = start_at
            + (&mut base)
                .take(u64::from_be_bytes(header_length_data))
                .read_to_end(&mut header_data)? as u64;
        // 此时指针在 Header 后，Data 前
        let serialize_options = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .allow_trailing_bytes()
            .with_limit(104857600 /* 100MiB */);
        let mut header: FRFSHeader = match serialize_options.deserialize(&header_data) {
            Ok(header) => header,
            Err(e) => {
                return Err(Error::DeserializationError(e.to_string()).into());
            }
        };
        // base header offset may not be zero
        // because it may be an embedded file.
        header.start_at = start_at + base.header.start_at;

        Ok(Self {
            header,
            base: base.source,
        })
    }

    // 在 FRFS 中递归打开路径里的文件的实现
    fn open_file(&self, current_dir: &Dir, mut path: path::Iter) -> Result<File> {
        let next_path = match path.next() {
            Some(str) => str.to_string_lossy().to_string(),
            None => return Err(Error::NotFound.into()),
        };
        if current_dir.files.contains_key(&next_path) {
            let file = current_dir
                .files
                .get(&next_path)
                .ok_or_else(|| Error::Unknown("contains key but no content".to_string()))?;
            let source = self.base.try_clone()?;
            // self.header.start_at + file.start_at 是这个 file 在 base 里的开始点
            Ok(File {
                header: FileHeader {
                    file_size: file.file_size,
                    start_at: self.header.start_at + file.start_at,
                },
                offset: 0, // Set the file cursor at start
                source,
            })
        } else if current_dir.dirs.contains_key(&next_path) {
            let dir = current_dir
                .dirs
                .get(&next_path)
                .ok_or_else(|| Error::Unknown("contains key but no content".to_string()))?;
            // 递归查找
            self.open_file(dir, path)
        } else {
            Err(Error::NotFound.into())
        }
    }

    // 在 FRFS 中递归打开路径里的文件夹的实现
    fn open_dir<'a>(current_dir: &'a Dir, mut path: path::Iter) -> Result<&'a Dir> {
        let next_path = match path.next() {
            Some(str) => str.to_string_lossy().to_string(),
            None => return Ok(current_dir),
        };
        if current_dir.dirs.contains_key(&next_path) {
            let dir = current_dir
                .dirs
                .get(&next_path)
                .ok_or_else(|| Error::Unknown("contains key but no content".to_string()))?;
            // 递归查找
            Self::open_dir(dir, path)
        } else {
            Err(Error::NotFound.into())
        }
    }

    /// Normalize the path, and strip the prefix `/`.
    fn normalize_and_strip(path: impl AsRef<Path>) -> PathBuf {
        let path = normalize_path(path.as_ref());
        if path.starts_with("/") {
            path.strip_prefix("/")
                .unwrap_or_else(|_| Path::new(""))
                .to_path_buf()
        } else {
            path
        }
    }

    /// Open the file, unix like path, with or without '/'
    /// at the beginning means starting from the root directory.
    pub fn open<P: AsRef<path::Path>>(&self, path: P) -> Result<File> {
        let path = Self::normalize_and_strip(path);
        self.open_file(&self.header.root, path.iter())
    }

    /// Returns the file (or folder) information of the corresponding path.
    pub fn metadata<P: AsRef<path::Path>>(&self, path: P) -> Result<Metadata> {
        let path = Self::normalize_and_strip(path);
        match self.open_file(&self.header.root, path.iter()) {
            Ok(file) => file.metadata(),
            Err(e) => match e.kind() {
                ErrorKind::NotFound => {
                    Self::open_dir(&self.header.root, path.iter())?;
                    Ok(Metadata(0, FileType(false)))
                }
                _ => Err(e),
            },
        }
    }

    /// Returns an iterator over the entries within a directory.
    pub fn read_dir<P: AsRef<path::Path>>(&self, path: P) -> Result<ReadDir> {
        let path = Self::normalize_and_strip(path);
        let dir = Self::open_dir(&self.header.root, path.iter())?;
        let mut dir_entrys: Vec<Result<DirEntry>> = dir
            .dirs
            .keys()
            .map(|dir_name| {
                let mut path = path.clone();
                path.push(dir_name);
                Ok(DirEntry {
                    path,
                    is_file: false,
                    file_size: 0,
                })
            })
            .collect();
        let dir_entrys_files: Vec<Result<DirEntry>> = dir
            .files
            .iter()
            .map(|(file_name, file)| {
                let mut path = path.clone();
                path.push(file_name);
                Ok(DirEntry {
                    path,
                    is_file: true,
                    file_size: file.file_size,
                })
            })
            .collect();
        dir_entrys.extend(dir_entrys_files);
        Ok(ReadDir { data: dir_entrys })
    }
}

#[derive(Debug)]
pub struct FRFSBuilder {
    header: FRFSHeader,
    source: PathBuf,
    magic_number_start: MagicNumber,
    magic_number_end: MagicNumber,
}

impl FRFSBuilder {
    // 从文件夹构造 FRFS 结构体，要求路径存在且为文件夹，有权限读取
    pub fn from_dir(source: impl AsRef<Path>) -> Result<Self> {
        Self::from_dir_with_header(source, MAGIC_NUMBER_START, MAGIC_NUMBER_END)
    }

    // 从文件夹构造 FRFS 结构体，要求路径存在且为文件夹，有权限读取
    pub fn from_dir_with_header(
        source: impl AsRef<Path>,
        magic_number_start: MagicNumber,
        magic_number_end: MagicNumber,
    ) -> Result<Self> {
        let source = source.as_ref();
        let mut dir = Dir {
            files: HashMap::new(),
            dirs: HashMap::new(),
        };
        let length = dir.fill_with(source, source)?;
        Ok(Self {
            header: FRFSHeader {
                root: dir,
                data_size: length,
                start_at: 0,
            },
            source: source.to_path_buf(),
            magic_number_start,
            magic_number_end,
        })
    }

    // 从 Dir 中读取文件路径`source`，不编码直接二进制写入 target，写入后增加 data_size(已写入的文件大小)
    fn fill_with_files(
        p: &Path,
        dir: &mut Dir,
        target: &mut Vec<u8>,
        mut data_size: u64,
    ) -> Result<u64> {
        for (name, file) in &mut dir.files {
            // 写入文件
            let mut buf = Vec::with_capacity(file.file_size as usize);
            let mut source = fs::File::open(p.join(name))?;
            let len = source.read_to_end(&mut buf)?;
            target.extend(buf);
            // 确定文件偏移
            file.start_at = data_size;
            file.file_size = len as u64;
            data_size += file.file_size;
        }
        for (name, sub_dir) in &mut dir.dirs {
            data_size = Self::fill_with_files(&p.join(name), sub_dir, target, data_size)?;
        }
        Ok(data_size)
    }

    // 将 FRFS 构建为 Vec<u8>
    pub fn build(mut self) -> Result<Vec<u8>> {
        // 构造数据体，需要比文件头先构建（构建数据时才知道数据头中的data_size），`+1024`防止溢出,
        // 这里的`data_size`仅用于参考创建 Vec 时的长度，实际数据大小为下面的填充后的`data_size`
        // 如果在调用完`from_dir`后，`build`前修改本地磁盘上的数据，则可能出现二者不一致的情况
        let mut data: Vec<u8> = Vec::with_capacity(self.header.data_size as usize + 1024);
        let data_size = Self::fill_with_files(&self.source, &mut self.header.root, &mut data, 0)?;
        // 此时`data_size`才是实际填充的数据大小
        self.header.data_size = data_size;
        // 限制文件头最大 100MiB
        let serialize_options = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .allow_trailing_bytes()
            .with_limit(104857600 /* 100MiB */);
        // 构建文件头
        let header = match serialize_options.serialize(&self.header) {
            Ok(vec) => vec,
            Err(e) => {
                return Err(Error::SerializationError(e.to_string()).into());
            }
        };

        // 构建 EmbedFS
        let mut target: Vec<u8> = Vec::new();
        // 写入 Magic Number
        target.extend(self.magic_number_start);
        // 写入文件头长度
        target.extend((header.len() as u64).to_be_bytes());
        // 写入 bincode 编码的文件头
        target.extend(&header);
        // 写入文件数据
        target.extend(&data);
        // 计算此时 target 长度并写入
        let target_length = target.len();
        // U64_LEN 是 target_length 的长度
        let target_length = target_length + U64_LEN;
        let target_length = target_length + self.magic_number_end.len();
        target.extend((target_length as u64).to_be_bytes());
        // 写入文件尾
        target.extend(self.magic_number_end);

        Ok(target)
    }
}

#[cfg(feature = "vfs")]
fn str_to_path(path: &str) -> PathBuf {
    let path = normalize_path(path.as_ref());
    if path.starts_with("/") {
        path.strip_prefix("/")
            .unwrap_or_else(|_| Path::new(""))
            .to_path_buf()
    } else {
        path
    }
}

#[cfg(feature = "vfs")]
impl vfs::filesystem::FileSystem for FRFS {
    fn read_dir(&self, path: &str) -> vfs::VfsResult<Box<dyn Iterator<Item = String> + Send>> {
        let path = str_to_path(path);
        let dir = Self::open_dir(&self.header.root, path.iter())?;
        let keys: Vec<String> = dir
            .dirs
            .keys()
            .map(|x| x.to_owned())
            .chain(dir.files.keys().map(|x| x.to_owned()))
            .collect();
        Ok(Box::new(keys.into_iter()))
    }

    fn create_dir(&self, _path: &str) -> vfs::VfsResult<()> {
        Err(vfs::error::VfsErrorKind::NotSupported.into())
    }

    fn open_file(&self, path: &str) -> vfs::VfsResult<Box<dyn vfs::SeekAndRead + Send>> {
        Ok(Box::new(self.open(path)?))
    }

    fn create_file(&self, _path: &str) -> vfs::VfsResult<Box<dyn io::Write + Send>> {
        Err(vfs::error::VfsErrorKind::NotSupported.into())
    }

    fn append_file(&self, _path: &str) -> vfs::VfsResult<Box<dyn io::Write + Send>> {
        Err(vfs::error::VfsErrorKind::NotSupported.into())
    }

    fn metadata(&self, path: &str) -> vfs::VfsResult<vfs::VfsMetadata> {
        let metadata = self.metadata(path)?;
        Ok(vfs::VfsMetadata {
            file_type: if metadata.is_dir() {
                vfs::VfsFileType::Directory
            } else {
                vfs::VfsFileType::File
            },
            len: metadata.len(),
        })
    }

    fn exists(&self, path: &str) -> vfs::VfsResult<bool> {
        match self.metadata(path) {
            Ok(_) => Ok(true),
            Err(e) => match e.kind() {
                io::ErrorKind::NotFound => Ok(false),
                _ => Err(vfs::VfsError::from(e)),
            },
        }
    }

    fn remove_file(&self, _path: &str) -> vfs::VfsResult<()> {
        Err(vfs::error::VfsErrorKind::NotSupported.into())
    }

    fn remove_dir(&self, _path: &str) -> vfs::VfsResult<()> {
        Err(vfs::error::VfsErrorKind::NotSupported.into())
    }
}

/// Load a FRFS file.
pub fn load(path: impl AsRef<Path>) -> Result<FRFS> {
    FRFS::new(path)
}

/// Load a FRFS file with header.
pub fn load_with_header(
    path: impl AsRef<Path>,
    magic_number_start: MagicNumber,
    magic_number_end: MagicNumber,
) -> Result<FRFS> {
    FRFS::new_with_header(path, magic_number_start, magic_number_end)
}

/// Load a FRFS file from frfs::File.
pub fn load_from_file(file: File) -> Result<FRFS> {
    FRFS::from_file(file)
}

/// Load a FRFS file from frfs::File with header.
pub fn load_from_file_with_header(
    file: File,
    magic_number_start: MagicNumber,
    magic_number_end: MagicNumber,
) -> Result<FRFS> {
    FRFS::from_file_with_header(file, magic_number_start, magic_number_end)
}

/// Pack a folder into a FRFS file.
pub fn pack(source: impl AsRef<Path>, target: impl AsRef<Path>) -> Result<()> {
    fs::write(target, FRFSBuilder::from_dir(source)?.build()?)?;
    Ok(())
}

/// Pack a folder into a FRFS file with header.
pub fn pack_with_header(
    source: impl AsRef<Path>,
    target: impl AsRef<Path>,
    magic_number_start: MagicNumber,
    magic_number_end: MagicNumber,
) -> Result<()> {
    fs::write(
        target,
        FRFSBuilder::from_dir_with_header(source, magic_number_start, magic_number_end)?.build()?,
    )?;
    Ok(())
}

// Normalize all intermediate components of the path
// Similar to fs::canonicalize() but doesn't resolve symlinks.
// From deno_core::normalize_path
// https://github.com/rust-lang/cargo/blob/af307a38c20a753ec60f0ad18be5abed3db3c9ac/src/cargo/util/paths.rs#L60-L85
#[inline]
fn normalize_path(path: &Path) -> PathBuf {
    let mut components = path.components().peekable();
    let mut ret = if let Some(c @ Component::Prefix(..)) = components.peek().cloned() {
        components.next();
        PathBuf::from(c.as_os_str())
    } else {
        PathBuf::new()
    };

    for component in components {
        match component {
            Component::Prefix(..) => unreachable!(),
            Component::RootDir => {
                ret.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                ret.pop();
            }
            Component::Normal(c) => {
                ret.push(c);
            }
        }
    }
    ret
}
