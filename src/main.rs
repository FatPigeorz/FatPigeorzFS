pub mod fs;
mod mkfs;

use clap::{Parser, Subcommand};
use fuser::Filesystem;
use std::path::PathBuf;

struct NullFS;

impl Filesystem for NullFS {}

#[derive(Parser, Debug)]
#[command(name = "FatPigeorzFS")]
#[command(author = "FatPigeorz <github.com/FatPigeorz>")]
#[command(version = "0.1.0")]
#[command(about = "A FileSystem based on Fuse and Rust", long_about = None)]
struct CLI {
    // subcommands
    #[command(subcommand)]
    commands: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Mkfs {
        // the image path
        #[arg(long, short, value_name = "IMAGE_PATH", default_value = "./myDisk.img")]
        path: PathBuf,

        // image size
        #[arg(long, short)]
        size: u32,
    },

    Mount {
        // mount point
        #[arg(long, short, value_name = "MOUNT_POINT", default_value = "./mnt")]
        path: PathBuf,

        // verbosity
        #[arg(short, value_name = "verbosity", default_value = "3")]
        verbosity: u32,

        // auto unmount on process exit
        #[arg(long, short)]
        auto_unmount: bool,
    },
}

fn main() {
    // env_logger::init();
    // let mountpoint = env::args_os().nth(1).unwrap();
    // fuser::mount2(NullFS, mountpoint, &[MountOption::AutoUnmount]).unwrap();
    let cli = CLI::parse();

    // match subcommands
    match cli.commands {
        Commands::Mkfs { path, size } => {
            // just print and raise not implementd
            println!("mkfs: path: {:?}, size: {}", path, size);
            mkfs::mkfs(path, size);
        }
        Commands::Mount {
            path,
            verbosity,
            auto_unmount,
        } => {
            // just print and raise not implementd
            println!(
                "mount: path: {:?}, verbosity: {}, auto_unmount: {}",
                path, verbosity, auto_unmount
            );
            unimplemented!();
        }
    }

    // let log_level = match cli.verbosity {
    //     0 => LevelFilter::Error,
    //     1 => LevelFilter::Warn,
    //     2 => LevelFilter::Info,
    //     3 => LevelFilter::Debug,
    //     _ => LevelFilter::Trace,
    // };

    // print log level
    // println!("log level: {:?}", log_level);

    // env_logger::builder()
    // .format_timestamp_nanos()
    // .filter_level(log_level)
    // .init();
}
