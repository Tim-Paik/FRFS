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
//! [load_from_embed_file]: Load a FRFS file from frfs::File.
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
const MAGIC_NUMBER_START: &[u8; 7] = b"FRFSv01";
const MAGIC_NUMBER_END: &[u8; 7] = b"FRFSEnd";
const USIZE_LEN: usize = usize::MAX.to_be_bytes().len();

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

// File: 内建文件类型
// size: 文件大小
// offset: 文件偏移（目前指针位置），无需序列化
// start_at: 文件在 source 中的开始位置
// mime: 文件类型的 MIMETYPE
// source: 文件源，无需序列化
/// The file type returned by the open function implements most of the APIs of std::fs::File
#[derive(Serialize, Deserialize, Debug)]
pub struct File {
    size: u64,
    #[serde(skip)]
    offset: u64,
    start_at: u64,
    mime: String,
    #[serde(skip)]
    source: Option<fs::File>,
}

impl File {
    #[inline]
    pub fn open<P: AsRef<path::Path>>(fs: &FRFS, path: P) -> Result<File> {
        fs.open(path)
    }
    #[inline]
    pub fn metadata(&self) -> Result<Metadata> {
        Ok(Metadata(self.size, FileType(true)))
    }
}

impl From<fs::File> for File {
    fn from(f: fs::File) -> Self {
        Self {
            size: f.metadata().unwrap().len(),
            offset: 0,
            start_at: 0,
            mime: "".to_string(),
            source: Some(f),
        }
    }
}

impl Read for File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let mut file = match &self.source {
            Some(f) => f,
            None => return Err(Error::NotFound.into()),
        };
        let ptr_now = file.seek(SeekFrom::Start(self.start_at + self.offset))?;
        let ptr_end = self.start_at + self.size;
        let ret = file.take(ptr_end - ptr_now).read(buf)?;
        self.offset += ret as u64;
        Ok(ret)
    }
}

impl Seek for File {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        match pos {
            SeekFrom::Current(offset) => self.seek(SeekFrom::Current(offset)),
            SeekFrom::Start(offset) => self.seek(SeekFrom::Start(self.start_at + offset)),
            SeekFrom::End(offset) => {
                let offset = if offset > 0 {
                    self.start_at + self.size
                } else {
                    self.start_at + self.size - offset.abs() as u64
                };
                self.seek(SeekFrom::Start(offset))
            }
        }
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
    #[inline]
    pub fn file_type(&self) -> FileType {
        self.1
    }
    #[inline]
    pub fn is_dir(&self) -> bool {
        self.file_type().is_dir()
    }
    #[inline]
    pub fn is_file(&self) -> bool {
        self.file_type().is_file()
    }
    #[inline]
    pub fn is_symlink(&self) -> bool {
        self.file_type().is_symlink()
    }
    #[inline]
    pub fn len(&self) -> u64 {
        self.0
    }
    #[inline]
    pub fn permissions(&self) -> Permissions {
        Permissions
    }
    #[inline]
    pub fn modified(&self) -> Result<std::time::SystemTime> {
        Ok(std::time::SystemTime::UNIX_EPOCH)
    }
    #[inline]
    pub fn accessed(&self) -> Result<std::time::SystemTime> {
        Ok(std::time::SystemTime::UNIX_EPOCH)
    }
    #[inline]
    pub fn created(&self) -> Result<std::time::SystemTime> {
        Ok(std::time::SystemTime::UNIX_EPOCH)
    }
}

/// Read Only
impl Permissions {
    #[inline]
    pub fn readonly(&self) -> bool {
        true
    }
}

impl FileType {
    #[inline]
    pub fn is_dir(&self) -> bool {
        !self.0
    }
    #[inline]
    pub fn is_file(&self) -> bool {
        self.0
    }
    #[inline]
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
    #[inline]
    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }

    #[inline]
    pub fn file_name(&self) -> OsString {
        self.path
            .file_name()
            .unwrap_or_else(|| OsStr::new(".."))
            .to_os_string()
    }

    #[inline]
    pub fn metadata(&self) -> Result<Metadata> {
        Ok(Metadata(self.file_size, self.file_type()?))
    }

    #[inline]
    pub fn file_type(&self) -> Result<FileType> {
        Ok(FileType(self.is_file))
    }
}

// Dir: 内建文件夹类型
// files: 此目录下文件
// dirs: 此目录下文件
#[derive(Serialize, Deserialize, Debug)]
struct Dir {
    files: HashMap<String, File>,
    dirs: HashMap<String, Dir>,
}

impl Dir {
    // 使用本地文件系统填充 Dir 结构体
    fn fill_with<P: AsRef<path::Path>>(
        &mut self,
        root: P,
        path: P,
        length: &mut u64,
    ) -> Result<()> {
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
                let source = fs::File::open(&path)?;
                let file = File {
                    size: source.metadata()?.len(),
                    offset: 0,
                    start_at: 0,
                    mime: mime_guess::from_path(&path)
                        .first_or_octet_stream()
                        .to_string(),
                    source: Some(fs::File::open(path.clone())?),
                };
                *length += file.size;
                self.files.insert(name, file);
            } else if path.is_dir() {
                // 构造子目录
                let mut dir = Dir {
                    files: HashMap::new(),
                    dirs: HashMap::new(),
                };
                // 填充子目录
                dir.fill_with(root.as_ref(), &path, length)?;
                self.dirs.insert(name, dir);
            }
        }
        Ok(())
    }
}

// FRFS: 只读文件系统类型
// root: 根目录'/', 用于存储文件所在位置
// data_size: 在 base 文件中的 start_at 后所跟的数据大小
// start_at: 在 base 文件中的开始点，无需序列化
// base: 文件源，无需序列化
/// Read-only File System
#[derive(Serialize, Deserialize, Debug)]
pub struct FRFS {
    root: Dir,
    data_size: u64,
    start_at: u64,
    #[serde(skip)]
    base: Option<fs::File>,
}

impl FRFS {
    /// Create FRFS data from filesystem.
    pub fn build_from_dir<P: AsRef<path::Path>>(source: P) -> Result<Vec<u8>> {
        let embed_fs = Self::from_dir(source)?;
        embed_fs.build()
    }

    // 从文件夹构造 FRFS 结构体，要求路径存在且为文件夹，有权限读取
    fn from_dir<P: AsRef<path::Path>>(source: P) -> Result<Self> {
        let source = source.as_ref();
        let mut length: u64 = 0;
        let mut dir = Dir {
            files: HashMap::new(),
            dirs: HashMap::new(),
        };
        dir.fill_with(source, source, &mut length)?;
        Ok(Self {
            root: dir,
            data_size: length, /* 此length仅用作参考 */
            start_at: 0,
            base: None,
        })
    }

    // 从 Dir 中读取文件路径`source`，不编码直接二进制写入 target，写入后增加 data_size(已写入的文件大小)
    fn fill_with_files(dir: &mut Dir, target: &mut Vec<u8>, data_size: &mut u64) -> Result<()> {
        for file in dir.files.values_mut() {
            let mut source = file.source.as_ref().ok_or_else(|| Error::NotFound)?;
            // 更新文件长度（调用完`from_dir`后，`build`前修改本地磁盘上的数据，文件长度可能变更）
            file.size = source.metadata()?.len();
            // 写入文件
            let mut buf = Vec::with_capacity(file.size as usize);
            source.read_to_end(&mut buf)?;
            target.extend(buf);
            // 确定文件偏移
            file.start_at = *data_size;
            *data_size += file.size;
        }
        for sub_dir in dir.dirs.values_mut() {
            Self::fill_with_files(sub_dir, target, data_size)?;
        }
        Ok(())
    }

    // 将 FRFS 构建为 Vec<u8>
    fn build(mut self) -> Result<Vec<u8>> {
        // 构造数据体，需要比文件头先构建（构建数据时才知道数据头中的data_size），`+1024`防止溢出,
        // 这里的`data_size`仅用于参考创建 Vec 时的长度，实际数据大小为下面的填充后的`data_size`
        // 如果在调用完`from_dir`后，`build`前修改本地磁盘上的数据，则可能出现二者不一致的情况
        let mut data: Vec<u8> = Vec::with_capacity(self.data_size as usize + 1024);
        let mut data_size: u64 = 0;
        Self::fill_with_files(&mut self.root, &mut data, &mut data_size)?;
        // 此时`data_size`才是实际填充的数据大小
        self.data_size = data_size;
        // 限制文件头最大 100MiB
        let serialize_options = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .allow_trailing_bytes()
            .with_limit(104857600 /* 100MiB */);
        // 构建文件头
        let header = match serialize_options.serialize(&self) {
            Ok(vec) => vec,
            Err(e) => {
                return Err(Error::SerializationError(e.to_string()).into());
            }
        };

        // 构建 EmbedFS
        let mut target: Vec<u8> = Vec::new();
        // 写入 Magic Number
        target.extend(MAGIC_NUMBER_START);
        // 写入文件头长度
        target.extend(&header.len().to_be_bytes());
        // 写入 bincode 编码的文件头
        target.extend(&header);
        // 写入文件数据
        target.extend(&data);
        // 计算此时 target 长度并写入
        let target_length = target.len();
        // U64_LEN 是 target_length 的长度
        let target_length = target_length + USIZE_LEN;
        let target_length = target_length + MAGIC_NUMBER_END.len();
        target.extend(target_length.to_be_bytes());
        // 写入文件尾
        target.extend(MAGIC_NUMBER_END);

        Ok(target)
    }

    /// Create frfs file from file system.
    pub fn pack<P: AsRef<path::Path>>(source: P, target: P) -> Result<()> {
        fs::write(target, Self::build_from_dir(source)?)?;
        Ok(())
    }

    /// Read FRFS from File.
    pub fn from_embed_file(mut file: File) -> Result<Self> {
        let mut base = tempfile::NamedTempFile::new()?;
        io::copy(&mut file, &mut base)?;
        Self::new(base.path())
    }

    /// Load a FRFS file.
    pub fn new<P: AsRef<path::Path> + Copy>(path: P) -> Result<Self> {
        let mut base = fs::File::open(path)?;
        let base_length = base.metadata()?.len();
        let mut magic_number_start_data = [0; MAGIC_NUMBER_START.len()];
        let mut header_length_data = [0; USIZE_LEN];
        let mut header_data = Vec::new();
        let mut data_length_data = [0; USIZE_LEN]; // 即创建文件时的 target_length 的 be_bytes 形式
        let mut magic_number_end_data = [0; MAGIC_NUMBER_END.len()];
        base.seek(SeekFrom::Start(base_length - MAGIC_NUMBER_END.len() as u64))?;
        // 此时指针指向 MAGIC_NUMBER_END 之前
        base.read_exact(&mut magic_number_end_data)?;
        if &magic_number_end_data != MAGIC_NUMBER_END {
            return Err(Error::IllegalData.into());
        }
        base.seek(SeekFrom::Start(
            base_length - MAGIC_NUMBER_END.len() as u64 - USIZE_LEN as u64,
        ))?;
        // 此时指针指向 data_length 之前
        base.read_exact(&mut data_length_data)?;
        base.seek(SeekFrom::Start(
            base_length - u64::from_be_bytes(data_length_data),
        ))?;
        // 此时指针指向 MAGIC_NUMBER_START
        base.read_exact(&mut magic_number_start_data)?;
        if &magic_number_start_data != MAGIC_NUMBER_START {
            return Err(Error::IllegalData.into());
        }
        base.read_exact(&mut header_length_data)?;
        // start_at 指 Data 段在 base 的位置
        let start_at = base.seek(SeekFrom::Current(0))?;
        let start_at = start_at
            + base
                .take(u64::from_be_bytes(header_length_data))
                .read_to_end(&mut header_data)? as u64;
        // 此时指针在 Header 后，Data 前
        let serialize_options = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .allow_trailing_bytes()
            .with_limit(104857600 /* 100MiB */);
        let mut frofs: Self = match serialize_options.deserialize(&header_data) {
            Ok(header) => header,
            Err(e) => {
                return Err(Error::DeserializationError(e.to_string()).into());
            }
        };
        frofs.base = Some(fs::File::open(path)?);
        frofs.start_at = start_at;

        Ok(frofs)
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
            let source = self
                .base
                .as_ref()
                .ok_or_else(|| Error::NotFound)?
                .try_clone()?;
            // self.start_at + file.start_at 是这个 file 在 base 里的开始点
            Ok(File {
                size: file.size,
                offset: 0, // 让File的文件指针指向0
                start_at: self.start_at + file.start_at,
                mime: file.mime.clone(),
                source: Some(source),
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
    fn open_dir<'a>(&self, current_dir: &'a Dir, mut path: path::Iter) -> Result<&'a Dir> {
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
            self.open_dir(dir, path)
        } else {
            Err(Error::NotFound.into())
        }
    }

    /// Open the file, unix like path, with or without '/'
    /// at the beginning means starting from the root directory.
    pub fn open<P: AsRef<path::Path>>(&self, path: P) -> Result<File> {
        let path = normalize_path(path.as_ref());
        let path = if path.starts_with("/") {
            path.strip_prefix("/")
                .unwrap_or_else(|_| Path::new(""))
                .to_path_buf()
        } else {
            path
        };
        self.open_file(&self.root, path.iter())
    }

    /// Returns the file (or folder) information of the corresponding path.
    pub fn metadata<P: AsRef<path::Path>>(&self, path: P) -> Result<Metadata> {
        let file = self.open(path)?;
        file.metadata()
    }

    /// Returns an iterator over the entries within a directory.
    pub fn read_dir<P: AsRef<path::Path>>(&self, path: P) -> Result<ReadDir> {
        let path = normalize_path(path.as_ref());
        let path = if path.starts_with("/") {
            path.strip_prefix("/")
                .unwrap_or_else(|_| Path::new(""))
                .to_path_buf()
        } else {
            path
        };
        let dir = self.open_dir(&self.root, path.iter())?;
        let mut dir_entrys: Vec<Result<DirEntry>> = dir
            .dirs
            .iter()
            .map(|(dir_name, _dir)| {
                let mut path = path.clone();
                path.push(dir_name.to_owned());
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
                    file_size: file.size,
                })
            })
            .collect();
        dir_entrys.extend(dir_entrys_files);
        Ok(ReadDir { data: dir_entrys })
    }
}

/// Load a FRFS file.
pub fn load<P: AsRef<path::Path> + Copy>(path: P) -> Result<FRFS> {
    FRFS::new(path)
}

/// Load a FRFS file from frfs::File.
pub fn load_from_embed_file(file: File) -> Result<FRFS> {
    FRFS::from_embed_file(file)
}

/// Pack a folder into a FRFS file.
pub fn pack<P: AsRef<path::Path>>(source: P, target: P) -> Result<()> {
    FRFS::pack(source, target)
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
