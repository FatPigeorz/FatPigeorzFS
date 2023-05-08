use fuser::{Filesystem};
use clap::{Parser};
use std::path::PathBuf;
use log::{LevelFilter};

struct NullFS;

impl Filesystem for NullFS {}

#[derive(Parser, Debug)]
#[command(name = "FatPigeorzFS")]
#[command(author = "FatPigeorz <github/FatPigeorz/")]
#[command(version = "1.0")]
#[command(about = "A FileSystem base on Fuse", long_about = None)]
struct CLI {
    #[arg(long, short, value_name="IMAGE_PATH", default_value = "./fs.img")]
    image_path: PathBuf,

    #[arg(long, short, value_name="MOUNTPOINT", default_value = "./mnt/")]
    mountpoint: PathBuf,
    
    #[arg(short, value_name="verbosity", default_value = "3")]
    verbosity: u32,
    
    #[arg(short, long, default_value = "true")]
    auto_unmount: bool
}

fn main() {
    // env_logger::init();
    // let mountpoint = env::args_os().nth(1).unwrap();
    // fuser::mount2(NullFS, mountpoint, &[MountOption::AutoUnmount]).unwrap();
    let cli = CLI::parse();
    // print the value of image_path
    println!("image_path: {:?}", cli.image_path);
    // print the value of mountpoint
    println!("mountpoint: {:?}", cli.mountpoint);
    // print the value of verbosity
    println!("verbosity: {:?}", cli.verbosity);
    // print the value of auto_unmount
    println!("auto_unmount: {:?}", cli.auto_unmount);
    
    let log_level = match cli.verbosity {
        0 => LevelFilter::Error,
        1 => LevelFilter::Warn,
        2 => LevelFilter::Info,
        3 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };
    
    // print log level
    println!("log level: {:?}", log_level);

    env_logger::builder()
    .format_timestamp_nanos()
    .filter_level(log_level)
    .init();
    


}
