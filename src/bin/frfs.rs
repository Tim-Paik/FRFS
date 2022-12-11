#[cfg(not(all(feature = "argh", feature = "anyhow")))]
compile_error!("Need bin feature to compile frfs cli");

use argh::FromArgs;

#[derive(FromArgs, PartialEq, Debug)]
/// Command line interface for frfs
struct Args {
    #[argh(subcommand)]
    cmd: Command,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand)]
enum Command {
    Pack(PackOpt),
    Unpack(UnpackOpt),
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "pack")]
/// Pack the folder into frfs
struct PackOpt {
    #[argh(positional)]
    /// path to the folder to pack
    src: String,
    #[argh(positional)]
    /// the path to the packaged frfs
    dst: String,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "unpack")]
/// Unpack frfs into a folder
struct UnpackOpt {
    #[argh(positional)]
    /// path to frfs to unpack
    src: String,
    #[argh(positional)]
    /// folder to unpack into
    dst: String,
}

fn unpack(
    fs: &frfs::FRFS,
    src_dir: std::path::PathBuf,
    dst_dir: std::path::PathBuf,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(&dst_dir)?;
    for entry in fs.read_dir(src_dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_file() {
            let mut src = fs.open(entry.path())?;
            let mut dst = std::fs::File::create(dst_dir.join(entry.file_name()))?;
            std::io::copy(&mut src, &mut dst)?;
        } else if meta.is_dir() {
            unpack(fs, entry.path(), dst_dir.join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Args = argh::from_env();
    match args.cmd {
        Command::Pack(opt) => {
            frfs::pack(opt.src, opt.dst)?;
        }
        Command::Unpack(opt) => {
            let fs = frfs::load(&opt.src)?;
            unpack(&fs, "/".into(), opt.dst.into())?;
        }
    }
    Ok(())
}
