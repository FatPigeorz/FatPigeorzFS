use std::{
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard}, cell::RefCell,
};

use once_cell::sync::Lazy;

use crate::fs::log::{log_begin, log_end};

use super::{
    fs::{BlockDevice, FileType, NFILE},
    inode::*,
};

#[derive(Default, Clone)]
pub struct FileInner {
    pub ty: FileType,
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
        Self(Mutex::new(vec![
            OpenFile(Arc::new(RefCell::new(FileInner::default())));
            NFILE as usize
        ]))
    }
}

pub static mut FTable: Lazy<FileTable> = Lazy::new(|| FileTable::new());

fn lock_table() -> MutexGuard<'static, Vec<OpenFile>> {
    unsafe { FTable.0.lock().unwrap() }
}

pub struct Stat {
    pub dev: u32, // always 0
    pub ino: u32, // inode number
    pub ty: FileType,
    pub nlink: u32, // number of links to inode in file system
    pub size: u32,
}

pub fn filealloc() -> Option<OpenFile> {
    let ft = unsafe { FTable.0.lock().unwrap() };
    ft.iter().find(|f| Arc::strong_count(&f.0) == 1).cloned()
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
    unsafe {(*file_ptr).ty = FileType::Free};
    log_begin();
    // the drop of inode will free the inode and put it into inode table
    unsafe {(*file_ptr).ip = None};
    log_end();
}

pub fn filestat(file: &OpenFile) -> Stat {
    let file = file.0.borrow();
    let ip = file.ip.as_ref().unwrap();
    ip.read_disk_inode(|diskinode| Stat {
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
    })
}

pub fn fileread(file: &OpenFile, dst: &mut [u8]) -> usize {
    let mut file_ptr = file.0.as_ptr();
    let n = rinode(
        unsafe {(*file_ptr).ip.as_mut().unwrap()},
        dst,
        unsafe {(*file_ptr).offset} as usize,
        dst.len(),
    );
    unsafe {(*file_ptr).offset += n as u32};
    n
}

pub fn filewrite(file: &OpenFile, src: &[u8]) -> usize {
    let mut file_ptr = file.0.as_ptr();
    let n = winode(
        unsafe {(*file_ptr).ip.as_mut().unwrap()},
        src,
        unsafe {(*file_ptr).offset} as usize,
        src.len(),
    );
    unsafe {(*file_ptr).offset += n as u32};
    n
}