mod fs;
mod mkfs;

use clap::{Parser, Subcommand};
use env_logger::{Builder, Target};
use fs::{file::{OpenFile, fopen, fread, fwrite}, filedisk::FileDisk, superblock::SB, log::LOG_MANAGER, fs::{BlockDevice, BLOCK_SIZE}, inode::{block_map, DirEntry}, buffer::get_buffer_block};
use std::{path::PathBuf, fs::{OpenOptions, File}, sync::Arc, io::Read};

use crate::fs::file::fstat;

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
    Shell {
        // the image path
        #[arg(long, short, value_name = "IMAGE_PATH", default_value = "./myDisk.img")]
        path: PathBuf,
    },
}

struct Shell {
    pub dev: Arc<dyn BlockDevice>,
    pub filetable: Vec<OpenFile>,
    pub cwd: PathBuf
}

impl Shell {
    pub fn new(image_path: PathBuf) -> Self {
        Builder::new()
            .is_test(true)
            .filter_level(log::LevelFilter::Error)
            .init();
        let file: File = OpenOptions::new()
            .read(true)
            .write(true)
            .create(false)
            .open("./test.img")
            .unwrap();
        let filedisk = Arc::new(FileDisk::new(file));
        unsafe { SB.init(filedisk.clone()) };
        unsafe { LOG_MANAGER.init(&SB, filedisk.clone()) };
        let root = fopen(filedisk.clone(), &PathBuf::from("/".to_string()));
        Self { dev: filedisk, filetable: vec![root], cwd: PathBuf::from("/".to_string()) }
    }

    pub fn eval(&mut self) {
        loop {
            // flush immediately
            print!("{} $ ", self.cwd.to_str().unwrap());
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).unwrap();
            let mut args = input.split_whitespace();
            let cmd = args.next().unwrap();
            // print prompt
            match cmd {
                "ls" => {
                    let path = match args.next() {
                        Some(path) => path,
                        None => self.cwd.to_str().unwrap()
                    };
                    self.ls(PathBuf::from(path));
                }
                "cat" => {
                    let path = args.next().unwrap();
                    self.cat(PathBuf::from(path));
                }
                "cd" => {
                    let path = args.next().unwrap();
                    self.cd(PathBuf::from(path));
                }
                "write" => {
                    let from = args.next().unwrap();
                    let to = args.next().unwrap();
                    self.write(PathBuf::from(from), PathBuf::from(to));
                }
                "mkdir" => {
                    let path = args.next().unwrap();
                    self.mkdir(PathBuf::from(path));
                }
                _ => {
                    println!("command not found: {}", cmd);
                }
            }
        }
    }

    fn ls(&self, path: PathBuf) {
        let mut ip = fopen(self.dev.clone(), &path).ip.unwrap();
        let mut entries = vec![];
        for i in (0..ip.read_disk_inode(|diskinode| {diskinode.size})).step_by(std::mem::size_of::<DirEntry>()) {
            let bn = block_map(&mut ip, i as u32 / BLOCK_SIZE);
            let entry = get_buffer_block(bn, self.dev.clone()).read().unwrap().read(
                i as usize % BLOCK_SIZE as usize,
                |entry : &DirEntry| {
                    *entry
                }
            );
            if entry.inum == 0 {
                continue;
            }
            entries.push(entry);
        }

        // file open and fstat
        for entry in entries {
            // join path and name 
            let name = String::from_utf8(Vec::from(entry.name)).unwrap();
            let fpath = PathBuf::from(self.cwd.clone()).join(name.clone());
            let mut file = fopen(self.dev.clone(), &fpath);
            let stat = fstat(&mut file);
            println!("name: {}\t size:{}\t name:{}", stat.name, stat.size,
                 match stat.type_ {
                    fs::fs::FileType::Dir => "Dir",
                    fs::fs::FileType::File => "File",
                    fs::fs::FileType::Free => "Free",
                 }
            );
        }
    }

    fn cat(&self, path: PathBuf) {
        let mut file = fopen(self.dev.clone(), &path);
        let mut dst = vec![0; 1024];
        while fread(&mut file, &mut dst) > 0 {
            println!("{}", String::from_utf8(dst.clone()).unwrap());
        }
    }

    fn cd(&mut self, path: PathBuf) {
        match path.to_str() {
            Some(".") => {
                return;
            }
            Some("..") => {
                self.cwd.pop();
                return;
            }
            _ => {
                self.cwd.push(path);
            }
        }
    }

    fn write (&mut self, from: PathBuf, to: PathBuf) {
        // from is the true file system
        // to is the virtual file system
        let mut from = std::fs::File::open(from).unwrap();
        let mut dst = vec![0; 1024];
        let mut to = fopen(self.dev.clone(), &to);
        while from.read(&mut dst).unwrap() > 0 {
            fwrite(&mut to, &dst);
        }
    }

    fn mkdir(&mut self, path: PathBuf) {
        let _ = fs::file::fcreat(self.dev.clone(), &path, crate::fs::fs::FileType::Dir);
    }
}


fn main() {
    // init builder
    let mut builder = Builder::new();
    // set log level
    builder.filter_level(log::LevelFilter::Info);
    let cli = CLI::parse();
    // match subcommands
    match cli.commands {
        Commands::Mkfs { path, size } => {
            // just print and raise not implementd
            println!("mkfs: path: {:?}, size: {}", path, size);
            mkfs::mkfs(path, size * 1024);
        }
        Commands::Shell { path } => 
            Shell::new(path).eval(),
    }
}


#[cfg(test)]
mod test {

    #[test]
    fn test_ls() {
        let shell = super::Shell::new(std::path::PathBuf::from("./test.img"));
        shell.ls(std::path::PathBuf::from("/"));
    }

    #[test]
    fn test_cat() {
        let shell = super::Shell::new(std::path::PathBuf::from("./test.img"));
        shell.cat(std::path::PathBuf::from("/test"));
    }
}
