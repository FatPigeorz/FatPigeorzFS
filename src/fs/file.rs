use std::{
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard}, cell::RefCell,
};

use once_cell::sync::Lazy;

use crate::fs::log::{log_begin, log_end};

use super::{
    fs::{BlockDevice, FileType, NFILE},
    inode::{*, self},
};

#[derive(Default, Copy, Clone, PartialEq)]
pub enum FDType {
    #[default] Free = 0,
    INODE = 1,
    // Device
    // PIPE
    // Socket
    // ...
}

#[derive(Default, Clone)]
pub struct FileInner {
    pub ty: FDType,
    pub readable: bool,
    pub writable: bool,
    pub offset: u32,
    pub path: PathBuf,
    pub ip: Option<InodePtr>,
    pub dev: Option<Arc<dyn BlockDevice>>,
}

#[derive(Default, Clone)]
pub struct OpenFile(pub Arc<RefCell<FileInner>>);

pub struct FileTable(Mutex<Vec<OpenFile>>);

impl FileTable {
    fn new() -> Self {
        // create a NFILE length vector
        Self(Mutex::new((0..NFILE).map(|_| OpenFile::default()).collect::<Vec<_>>()))
    }
}

pub static mut FTABLE: Lazy<FileTable> = Lazy::new(|| FileTable::new());

fn lock_table() -> MutexGuard<'static, Vec<OpenFile>> {
    unsafe { FTABLE.0.lock().unwrap() }
}

pub struct Stat {
    pub dev: u32, // always 0
    pub ino: u32, // inode number
    pub ty: FileType,
    pub nlink: u32, // number of links to inode in file system
    pub size: u32,
}

#[derive(Debug, PartialEq)]
pub enum OpenMode {
    ORdonly,
    OWronly,
    ORdwr,
    OCreate,
    OTrunc,
}

pub fn filealloc() -> Option<OpenFile> {
    let ft = lock_table();
    ft.iter().find(|f| Arc::strong_count(&f.0) == 1).cloned()
}

/// path should be absolute path
pub fn fileopen(dev: Arc<dyn BlockDevice>, path: &PathBuf, omod: OpenMode) -> Result<OpenFile, String> {
    // if exists in table
    {
        let ft = unsafe { FTABLE.0.lock().unwrap() };
        if let Some(f) = ft.iter().find(|f| f.0.borrow().path == *path) {
            return Ok(f.clone());
        }
    }
    // find inode
    let ip;
    log_begin();
    if omod == OpenMode::OCreate {
        ip = inode::create(dev.clone(), &path, FileType::File);
        if ip.is_none() {
            log_end();
            return Err("file exists".to_string());
        }
    } else {
        ip = inode::find_inode(dev.clone(), &path);
        if ip.is_none() {
            log_end();
            return Err("file not found".to_string());
        }
        // check mode
        if ip.as_ref().unwrap().read_disk_inode(
            |diskinode| {
                omod != OpenMode::ORdonly && diskinode.ftype == 2
            }
        )  {
            log_end();
            return Err("file is a directory".to_string());
        } 
        if omod == OpenMode::OTrunc {
            ip.as_ref().unwrap().modify_disk_inode(|diskinode| {
                diskinode.size = 0;
                Inode::truncate(dev.clone(), diskinode);
            });
        }
    }
    log_end();
    // alloc file 
    let file = filealloc();
    if file.is_none() {
        return Err("no free file in table".to_string());
    }
    let file = file.unwrap();
    let mut file_ptr = file.0.as_ptr();
    unsafe {
        (*file_ptr).ty = FDType::INODE;
        (*file_ptr).readable = omod == OpenMode::ORdonly || omod == OpenMode::ORdwr;
        (*file_ptr).writable = omod == OpenMode::OWronly || omod == OpenMode::ORdwr;
        (*file_ptr).offset = 0;
        (*file_ptr).path = path.clone();
        (*file_ptr).ip = ip;
        (*file_ptr).dev = Some(dev);
    }

    Ok(file)
}

pub fn mkdir(dev: Arc<dyn BlockDevice>, path: &PathBuf) {
    log_begin();
    inode::create(dev.clone(), path, FileType::Dir);
    log_end();
}

// the owner ship should move to here directly
// do not clone the Arc pointer
pub fn fileclose(file: OpenFile) {
    let _ftable = lock_table();
    assert!(Arc::strong_count(&file.0) > 1);
    if Arc::strong_count(&file.0) >= 2 {
        return;
    }
    // clear attribute
    // ty = FileType::Free;
    let file_ptr = file.0.as_ptr();
    unsafe {(*file_ptr).ty = FDType::Free};
    log_begin();
    // the drop of inode will free the inode and put it into inode table
    unsafe {(*file_ptr).ip = None};
    log_end();
}

pub fn filestat(file: &OpenFile) -> Stat {
    let file = file.0.borrow();
    let ip = file.ip.as_ref().unwrap();
    log_begin();
    let ret = ip.read_disk_inode(|diskinode| Stat {
        dev: 0,
        ino: ip.0.inum,
        ty: match diskinode.ftype {
            0 => FileType::Free,
            1 => FileType::File,
            2 => FileType::Dir,
            _ => panic!("unknown file type"),
        },
        nlink: diskinode.nlink as u32,
        size: diskinode.size,
    });
    log_end();
    ret
}

pub fn fileread(file: &OpenFile, dst: &mut [u8]) -> usize {
    let mut file_ptr = file.0.as_ptr();
    log_begin();
    let n = rinode(
        unsafe {(*file_ptr).ip.as_mut().unwrap()},
        dst,
        unsafe {(*file_ptr).offset} as usize,
        dst.len(),
    );
    log_end();
    unsafe {(*file_ptr).offset += n as u32};
    n
}

pub fn filewrite(file: &OpenFile, src: &[u8]) -> usize {
    let mut file_ptr = file.0.as_ptr();
    log_begin();
    let n = winode(
        unsafe {(*file_ptr).ip.as_mut().unwrap()},
        src,
        unsafe {(*file_ptr).offset} as usize,
        src.len(),
    );
    log_end();
    unsafe {(*file_ptr).offset += n as u32};
    n
}

pub fn fileunlink(dev: Arc<dyn BlockDevice>, path: &PathBuf) -> Result<(), String> {
    log_begin();
    let dp = find_parent_inode(dev.clone(), path);
    if dp.is_none() {
        return Err("fileunlink: cannot find parent inode".to_string());
    }
    let mut dp = dp.unwrap();
    let name = path.file_name().unwrap().to_str().unwrap(); 
    dirunlink(&mut dp, name)?;
    dp.modify_disk_inode(
        |diskinode| {
            diskinode.nlink -= 1;
        }
    );
    log_end();
    Ok(())
}