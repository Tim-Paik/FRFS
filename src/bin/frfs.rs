use argh::FromArgs;
use frfs::MagicNumber;
use std::io::Result;

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
    #[argh(option, from_str_fn(parse_magic_number))]
    /// custom magic number start
    magic_number_start: Option<MagicNumber>,
    #[argh(option, from_str_fn(parse_magic_number))]
    /// custom magic number start
    magic_number_end: Option<MagicNumber>,
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
    #[argh(option, from_str_fn(parse_magic_number))]
    /// custom magic number start
    magic_number_start: Option<MagicNumber>,
    #[argh(option, from_str_fn(parse_magic_number))]
    /// custom magic number start
    magic_number_end: Option<MagicNumber>,
}

fn parse_magic_number(value: &str) -> std::result::Result<MagicNumber, String> {
    value
        .as_bytes()
        .try_into()
        .map_err(|_| "magic number should be a [u8; 7]".to_string())
}

fn unpack(fs: &frfs::FRFS, src_dir: std::path::PathBuf, dst_dir: std::path::PathBuf) -> Result<()> {
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

fn main() -> Result<()> {
    let args: Args = argh::from_env();
    match args.cmd {
        Command::Pack(opt) => {
            if opt.magic_number_start.is_none() || opt.magic_number_end.is_none() {
                if opt.magic_number_start.is_some() || opt.magic_number_end.is_some() {
                    println!("WARN: magic_number_start and magic_number_end need to be provided at the same time, magic_number will be ignored");
                }
                frfs::pack(opt.src, opt.dst)?;
            } else {
                frfs::pack_with_header(
                    opt.src,
                    opt.dst,
                    opt.magic_number_start.unwrap(),
                    opt.magic_number_end.unwrap(),
                )?;
            }
        }
        Command::Unpack(opt) => {
            let fs = if opt.magic_number_start.is_none() || opt.magic_number_end.is_none() {
                if opt.magic_number_start.is_some() || opt.magic_number_end.is_some() {
                    println!("WARN: magic_number_start and magic_number_end need to be provided at the same time, magic_number will be ignored");
                }
                frfs::load(&opt.src)?
            } else {
                frfs::load_with_header(
                    &opt.src,
                    opt.magic_number_start.unwrap(),
                    opt.magic_number_end.unwrap(),
                )?
            };
            unpack(&fs, "/".into(), opt.dst.into())?;
        }
    }
    Ok(())
}
