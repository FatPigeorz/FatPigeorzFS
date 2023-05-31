mod fs;
mod mkfs;

use clap::{Parser, Subcommand};
use env_logger::{Builder};
use fs::{
    buffer::{get_buffer_block, sync_all},
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

use crate::fs::{file::{filestat, fileclose}, fs::FileType};

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
    #[allow(unused)]
    pub filetable: Vec<OpenFile>,
    pub cwd: PathBuf,
}

fn canonicalize(path: PathBuf) -> PathBuf {
    // eliminate the . and .. in the path
    let mut stack = Vec::new();
    // root
    stack.push(std::path::Component::RootDir);
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => stack.push(component),
            std::path::Component::ParentDir => {
                stack.pop();
            }
            _ => {}
        }
    }
    // if empty, is the root
    if stack.is_empty() {
        stack.push(std::path::Component::RootDir);
    }
    stack.iter().fold(PathBuf::new(), |mut acc, x| {
        acc.push(x);
        acc
    })
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
                "exit" => {
                    break;
                }
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
                "mkdir" => {
                    let path = args.next().unwrap();
                    self.mkdir(PathBuf::from(path));
                }
                "touch" => {
                    let path = args.next().unwrap();
                    self.touch(PathBuf::from(path));
                }
                "rm" => {
                    let path = args.next().unwrap();
                    self.rm(PathBuf::from(path));
                }
                "rmdir" => {
                    let path = args.next().unwrap();
                    self.rmdir(PathBuf::from(path));
                }
                _ => {
                    println!("command not found: {}", cmd);
                }
            }
        }
        sync_all();
    }

    fn ls(&self, path: PathBuf) {
        let fd = fileopen(self.dev.clone(), &path, OpenMode::ORdonly).unwrap();
        let mut entries = vec![];
        // print header
        let mut entry = [0u8; std::mem::size_of::<DirEntry>()];
        while fileread(&fd, &mut entry) > 0 {
            entries.push(unsafe { std::mem::transmute::<[u8; std::mem::size_of::<DirEntry>()], DirEntry>(entry) });
        }
        println!("{:<12} {:<12} {:<12} {:<12}", "name", "type", "size", "nlink");

        // file open and fstat
        for entry in entries {
            let name = std::str::from_utf8(entry.name.as_slice()).unwrap().trim_matches(char::from(0));
            // canonicalize the path
            let fpath = canonicalize(PathBuf::from(path.clone()).join(name));
            let mut file = fileopen(self.dev.clone(), &fpath, OpenMode::ORdonly).unwrap();
            let stat = filestat(&mut file);
            // print
            println!(
                "{:<12} {:<12} {:<12} {:<12}",
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
        fileclose(fd);
    }

    fn cat(&self, path: PathBuf) {
        let mut fd = fileopen(self.dev.clone(), &path, OpenMode::ORdonly).unwrap();
        let mut dst = vec![0; 1024];
        while fileread(&mut fd, &mut dst) > 0 {
            print!("{}", String::from_utf8(dst.clone()).unwrap());
            dst.fill(0);
        }
        fileclose(fd);
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
        loop {
            let n = from.read(&mut dst).unwrap();
            filewrite(&mut to, &dst[0..n]);
            if n < 1024 {
                break;
            }
        }
        fileclose(to);
    }

    fn mkdir(&mut self, path: PathBuf) {
        match fs::file::mkdir(self.dev.clone(), &path) {
            Ok(_) => {}
            Err(e) => {
                println!("mkdir: {}", e);
            }
        }
    }

    fn touch(&mut self, path: PathBuf) {
        match fs::file::fileopen(self.dev.clone(), &path, OpenMode::OCreate) {
            Ok(_) => {}
            Err(e) => {
                println!("touch: {}", e);
            }
        }
    }

    fn rm(&mut self, path: PathBuf) {
        fs::file::fileunlink(self.dev.clone(), &path).unwrap();
    }

    fn rmdir(&mut self, path: PathBuf) {
        // unlink all files in dir
        let fd = fileopen(self.dev.clone(), &path, OpenMode::ORdonly).unwrap();
        let ip = unsafe { (*fd.0.as_ptr()).ip.as_ref().unwrap() };
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
            // unlink
            fs::file::fileunlink(self.dev.clone(), &fpath).unwrap();
        }
        // unlink dir
        fs::file::fileunlink(self.dev.clone(), &path).unwrap();
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
        Commands::Shell { path } => Shell::new(path).repr(),
    }
}

#[cfg(test)]
mod test {
    use crate::canonicalize;

    #[test]
    fn test_canonicalize() {
        let path = std::path::PathBuf::from("/usr/bin/../bin/./ls");
        assert_eq!(
            canonicalize(path.clone()),
            std::path::PathBuf::from("/usr/bin/ls")
        );
        println!("{:?}", canonicalize(path));
    }

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

    #[test]
    fn test_touch() {
        let mut shell = super::Shell::new(std::path::PathBuf::from("./test.img"));
        shell.touch(std::path::PathBuf::from("/test"));
        shell.ls(std::path::PathBuf::from("/"));
    }

    #[test]
    fn test_mkdirs() {
        let mut shell = super::Shell::new(std::path::PathBuf::from("./test.img"));
        shell.mkdir(std::path::PathBuf::from("/bin"));
        shell.mkdir(std::path::PathBuf::from("/etc"));
        shell.mkdir(std::path::PathBuf::from("/home"));
        shell.mkdir(std::path::PathBuf::from("/home/texts"));
        shell.mkdir(std::path::PathBuf::from("/home/reports"));
        shell.mkdir(std::path::PathBuf::from("/home/photos"));
        shell.mkdir(std::path::PathBuf::from("/dev"));
        shell.ls(std::path::PathBuf::from("/"));
        println!("");
        shell.ls(std::path::PathBuf::from("/home/"));
        println!("");
        shell.touch(std::path::PathBuf::from("/home/texts/text1"));
        println!("");
        shell.ls(std::path::PathBuf::from("/home/texts"));
        println!("");
        // write bigphoto to
        shell.touch(std::path::PathBuf::from("/home/photos/nishino.jpg"));
        shell.write( 
            std::path::PathBuf::from("./nishino.jpg"),
            std::path::PathBuf::from("/home/photos/nishino.jpg"),
        );
        shell.ls(std::path::PathBuf::from("/home/photos"));
    }
}
