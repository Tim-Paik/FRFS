use std::{fs::File, io::Result};

#[cfg(windows)]
pub fn read_at(f: &File, buf: &mut [u8], offset: u64) -> Result<usize> {
    use std::os::windows::fs::FileExt;
    f.seek_read(buf, offset)
}

#[cfg(unix)]
pub fn read_at(f: &File, buf: &mut [u8], offset: u64) -> Result<usize> {
    use std::os::unix::fs::FileExt;
    f.read_at(buf, offset)
}

#[cfg(target_os = "wasi")]
pub fn read_at(f: &File, buf: &mut [u8], offset: u64) -> Result<usize> {
    use std::os::wasi::fs::FileExt;
    f.read_at(buf, offset)
}
