use vfs::*;

fn main() -> VfsResult<()> {
    // Pack a frfs file
    frfs::pack("src", "./src.frfs")?;
    // then open it
    let fs: VfsPath = frfs::load("./src.frfs")?.into();
    // iterating over and printing what's inside is as easy as using std::fs
    for entry in fs.read_dir()? {
        match entry.metadata()?.file_type {
            VfsFileType::File => {
                println!("File {{ file: {:?} }}", entry.as_str());
                let mut buf = String::new();
                entry.open_file()?.read_to_string(&mut buf)?;
                println!("{}", buf);
            }
            VfsFileType::Directory => println!("Dir {{ path: {:?} }}", entry.as_str()),
        }
    }
    std::fs::remove_file("./src.frfs")?;
    Ok(())
}
