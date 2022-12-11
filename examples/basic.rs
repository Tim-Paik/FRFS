fn main() -> std::io::Result<()> {
    // Pack a frfs file
    frfs::pack("src", "./src.frfs")?;
    // then open it
    let fs = frfs::load("./src.frfs")?;
    // iterating over and printing what's inside is as easy as using std::fs
    for entry in fs.read_dir("/")? {
        let entry = entry?;
        if entry.metadata()?.is_file() {
            println!(
                "File {{ file: {:?}, inner: {:?} }}",
                entry.path(),
                fs.open(entry.path())?
            );
        } else if entry.metadata()?.is_dir() {
            println!("Dir {{ path: {:?} }}", entry.path());
        }
    }
    std::fs::remove_file("./src.frfs")?;
    Ok(())
}
