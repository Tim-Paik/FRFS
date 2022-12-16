use std::{fs::File, io::Result};

#[cfg(windows)]
pub fn read_at(f: &File, buf: &mut [u8], offset: u64) -> Result<usize> {
    use std::os::windows::fs::FileExt;
    // Note we use `seek_read` here to imitate `read_at`.
    //
    // `read_at` is `pread`/`pread64` on Unix, which will not update file
    // cursor after reading. However on Windows, all reading operations
    // is finally `NtReadFile`, which will certainly update the file cursor.
    //
    // We use `seek_read` here because we don't care about the file cursor
    // at all. We just need an atomic operation to seek and read.
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
