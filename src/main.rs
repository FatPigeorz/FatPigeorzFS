mod fs;
mod mkfs;

use clap::{Parser, Subcommand};
use env_logger::{Builder};
use fs::{
    buffer::get_buffer_block,
    file::{fileopen, fileread, filewrite, OpenFile, OpenMode},
    filedisk::FileDisk,
    fs::{BlockDevice, BLOCK_SIZE},
    inode::{block_map, DirEntry},
    log::LOG_MANAGER,
    superblock::SB,
};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    sync::Arc,
};

use crate::fs::{file::{filestat, FileTable}, fs::FileType};

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
    pub cwd: PathBuf,
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
            .open(image_path)
            .unwrap();
        let filedisk = Arc::new(FileDisk::new(file));
        unsafe { SB.init(filedisk.clone()) };
        unsafe { LOG_MANAGER.init(&SB, filedisk.clone()) };
        let root = fileopen(
            filedisk.clone(),
            &PathBuf::from("/".to_string()),
            OpenMode::ORdonly,
        );
        Self {
            dev: filedisk,
            filetable: vec![root.unwrap()],
            cwd: PathBuf::from("/".to_string()),
        }
    }

    pub fn repr(&mut self) {
        loop {
            // flush immediately
            print!("{} $ ", self.cwd.to_str().unwrap());
            std::io::stdout().flush().unwrap();
            let input = String::new();
            let mut args = input.split_whitespace();
            let cmd = args.next().unwrap();
            // print prompt
            match cmd {
                "ls" => {
                    let path = match args.next() {
                        Some(path) => PathBuf::from(self.cwd.clone()).join(path),
                        None => self.cwd.clone(),
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
                // "mkdir" => {
                //     let path = args.next().unwrap();
                //     self.mkdir(PathBuf::from(path));
                // }
                _ => {
                    println!("command not found: {}", cmd);
                }
            }
        }
    }

    fn ls(&self, path: PathBuf) {
        let fd = fileopen(self.dev.clone(), &path, OpenMode::ORdonly).unwrap();
        let ip = unsafe { (*fd.0.as_ptr()).ip.as_ref().unwrap() };
        let mut entries = vec![];
        for i in (0..ip.read_disk_inode(|diskinode| diskinode.size))
            .step_by(std::mem::size_of::<DirEntry>())
        {
            let bn = ip.modify_disk_inode(|diskinode| {
                block_map(diskinode, self.dev.clone(), i as u32 / BLOCK_SIZE)
            });
            let entry = get_buffer_block(bn, self.dev.clone())
                .read()
                .unwrap()
                .read(i as usize % BLOCK_SIZE as usize, |entry: &DirEntry| *entry);
            if entry.inum == 0 {
                continue;
            }
            entries.push(entry);
        }
        
        // print header
        println!("{:<8} {:<8} {:<8} {:<8}", "name", "type", "size", "nlink");

        // file open and fstat
        for entry in entries {
            let name = std::str::from_utf8(entry.name.as_slice()).unwrap().trim_matches(char::from(0));
            // handle . and ..
            let fpath = match name {
                "." => self.cwd.clone(),
                ".." => {
                    // handle "/"
                    if self.cwd.to_str().unwrap() == "/" {
                        PathBuf::from("/".to_string())
                    } else {
                        PathBuf::from(self.cwd.clone()).parent().unwrap().to_path_buf()
                    }
                }
                _ => PathBuf::from(self.cwd.clone()).join(name),
            };
            let mut file = fileopen(self.dev.clone(), &fpath, OpenMode::ORdonly).unwrap();
            let stat = filestat(&mut file);
            // print
            println!(
                "{:<8} {:<8} {:<8} {:<8}",
                name,
                match stat.ty {
                    FileType::Free => "free",
                    FileType::File => "file",
                    FileType::Dir => "dir",
                },
                stat.size,
                stat.nlink
            );
        }
    }

    fn cat(&self, path: PathBuf) {
        let mut fd = fileopen(self.dev.clone(), &path, OpenMode::ORdonly).unwrap();
        let mut dst = vec![0; 1024];
        while fileread(&mut fd, &mut dst) > 0 {
            println!("{}", String::from_utf8(dst.clone()).unwrap());
        }
    }

    fn cd(&mut self, path: PathBuf) {
        // iter and change cwd
        let mut path = path;
        if path.starts_with("/") {
            self.cwd = PathBuf::from("/");
            path = path.strip_prefix("/").unwrap().to_path_buf();
        }
        for name in path.iter() {
            if name == "." || (name == ".." && self.cwd == PathBuf::from("/")) {
                continue;
            } else if name == ".." {
                self.cwd.pop();
            } else {
                self.cwd.push(name);
            }
        }
    }

    fn write(&mut self, from: PathBuf, to: PathBuf) {
        // from is the true file system
        // to is the virtual file system
        let mut from = std::fs::File::open(from).unwrap();
        let mut dst = vec![0; 1024];
        let mut to = fileopen(self.dev.clone(), &to, OpenMode::OWronly).unwrap();
        while from.read(&mut dst).unwrap() > 0 {
            filewrite(&mut to, &dst);
        }
    }

    // fn mkdir(&mut self, path: PathBuf) {
    //     let _ = fcreat(self.dev.clone(), &path, crate::fs::fs::FileType::Dir);
    // }
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
        Commands::Shell { path } => Shell::new(path).repr(),
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
