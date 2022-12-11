use std::io::Read;

fn main() -> std::io::Result<()> {
    // Pack a frfs file
    frfs::pack("examples", "./src/src.frfs")?;
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
        if entry.path().as_os_str() == "src.frfs" {
            let fs = frfs::load_from_file(fs.open(entry.path())?)?;
            for entry in fs.read_dir("/")? {
                let entry = entry?;
                if entry.metadata()?.is_file() {
                    println!(
                        "File {{ file: {:?}, inner: {:?} }}",
                        entry.path(),
                        fs.open(entry.path())?
                    );
                    let mut buf = String::new();
                    fs.open(entry.path())?.read_to_string(&mut buf)?;
                    println!("{}", buf);
                } else if entry.metadata()?.is_dir() {
                    println!("Dir {{ path: {:?} }}", entry.path());
                }
            }
        }
    }
    std::fs::remove_file("./src.frfs")?;
    std::fs::remove_file("./src/src.frfs")?;
    Ok(())
}
