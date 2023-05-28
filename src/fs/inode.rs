use core::panic;
use std::cell::RefCell;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use log::info;
use once_cell::sync::Lazy;

use crate::fs::fs::BLOCK_SIZE;

use super::fs::{NINDIRECT, NINODES, ROOTINO};
use super::log::log_write;
use super::{
    buffer::get_buffer_block,
    fs::{BlockDevice, FileType, BPB, IPB, NAMESIZE, NDIRECT},
    superblock::SB,
};

// Disk Struct
#[repr(C)]
#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub struct DiskInode {
    pub dev: u32,                           // Device number, always 0
    pub ftype: u16,                         // File type
    pub nlink: u16,                         // Number of links to file
    pub size: u32,                          // Size of file (bytes)
    pub addrs: [u32; NDIRECT as usize + 1], // Pointers to blocks
}

// directory contains a sequence of entry
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct DirEntry {
    pub inum: u32,
    pub name: [u8; NAMESIZE as usize],
}

pub fn namecmp(s: &[u8], t: &String) -> bool {
    let mut i = 0;
    for c in t.chars() {
        if i >= s.len() {
            return false;
        }
        if s[i] != c as u8 {
            return false;
        }
        i += 1;
    }
    true
}

pub fn nameassign(s: &mut [u8], t: &String) {
    let mut i = 0;
    for c in t.chars() {
        if i >= s.len() {
            panic!("nameassign: name too long");
        }
        s[i] = c as u8;
        i += 1;
    }
    while i < s.len() {
        s[i] = 0;
        i += 1;
    }
}

// get the (block,offset) of inum
fn addr_of_inode(inum: u32) -> (u32, u32) {
    (inum / IPB + unsafe { SB.inodestart }, inum % IPB * std::mem::size_of::<DiskInode>() as u32)
}

// get the block containing the bitmap
fn block_of_bitmap(block: u32) -> u32 {
    block / BPB + unsafe { SB.bmapstart }
}

fn balloc(dev: Arc<dyn BlockDevice>) -> Option<u32> {
    for b in (0..unsafe { SB.size }).step_by(BPB as usize) {
        let bno = block_of_bitmap(b);
        let blk = get_buffer_block(bno, dev.clone());
        let mut guard = blk.write().unwrap();
        let mut buf = guard.read(0, |buf: &[u8; BLOCK_SIZE as usize]| *buf);
        for bi in 0..BPB as usize {
            let m = 1 << (bi % 8);
            if buf[bi / 8] & m == 0 {
                buf[bi / 8] |= m;
                log_write(
                    &mut guard,
                    bi / 8,
                    |data: &mut [u8; BLOCK_SIZE as usize]| {
                        data[bi / 8] = buf[bi / 8];
                    },
                );
                log_write(
                    &mut get_buffer_block(bi as u32 + b, dev.clone())
                        .write()
                        .unwrap(),
                    0,
                    |data: &mut [u8; BLOCK_SIZE as usize]| {
                        data.fill(0);
                    },
                );
                return Some(b + bi as u32);
            }
        }
    }
    None
}

fn bfree(dev: Arc<dyn BlockDevice>, b: u32) {
    let bno = block_of_bitmap(b);
    let bi = b % BPB;
    get_buffer_block(bno, dev.clone())
        .write()
        .unwrap()
        .sync_write(bi as usize / 8, |data: &mut u8| {
            *data &= !(1 << (bi % 8));
        });
}

pub struct Inode {
    pub dev: Option<Arc<dyn BlockDevice>>,
    pub inum: u32,
    pub dinode: Mutex<Option<DiskInode>>, // inode copy
}

impl Inode {
    pub fn new() -> Self {
        Self {
            dev: None,
            inum: 0,
            dinode: Mutex::new(None),
        }
    }
}

impl Inode {
    fn read_disk_inode<V>(&self, f: impl FnOnce(&DiskInode) -> V) -> V {
        let (blk, off) = addr_of_inode(self.inum);
        get_buffer_block(blk, self.dev.as_ref().unwrap().clone())
            .read()
            .unwrap()
            .read(off as usize, f)
    }

    fn modify_disk_inode<V>(&self, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        let (blk, off) = addr_of_inode(self.inum);
        log_write(
            &mut get_buffer_block(blk, self.dev.as_ref().unwrap().clone())
                .write()
                .unwrap(),
            off as usize,
            f,
        )
    }

    pub fn truncate(dev: Arc<dyn BlockDevice>, dinode: &mut DiskInode) {
        // free the data blocks
        for i in 0..NDIRECT {
            if dinode.addrs[i as usize] != 0 {
                bfree(dev.clone(), dinode.addrs[i as usize]);
                dinode.addrs[i as usize] = 0;
            }
        }

        if dinode.addrs[NDIRECT as usize] > 0 {
            // read the indirect block
            let addrs = get_buffer_block(dinode.addrs[NDIRECT as usize], dev.clone())
                .read()
                .unwrap()
                .read(0, |addrs: &[u32; NINDIRECT as usize]| *addrs);
            for i in 0..NINDIRECT as usize {
                if addrs[i] != 0 {
                    bfree(dev.clone(), addrs[i as usize]);
                }
            }
            bfree(dev.clone(), dinode.addrs[NDIRECT as usize]);
            dinode.addrs[NDIRECT as usize] = 0;
        }
    }
}

// design object:
// InodePtr is a pointer to Inode
// Every File should have a InodePtr
// Many file many have a ptr to same inode
pub struct InodePtr(Arc<Inode>);

impl InodePtr {
    pub fn new() -> Self {
        Self(Arc::new(Inode::new()))
    }

    pub fn read_disk_inode<V>(&mut self, f: impl FnOnce(&DiskInode) -> V) -> V {
        // if the disk inode is not loaded, load it
        let mut guard = self.0.dinode.lock().unwrap();
        if guard.is_none() {
            let dinode = self.0.read_disk_inode(|dinode_ref| *dinode_ref);
            // self.0.dinode = Some(Mutex::new(dinode));
            // pass the borrow checker
            // use unsafe
            *guard = Some(dinode);
        }
        f(guard.as_ref().unwrap())
    }
}

pub struct InodePtrManager(Mutex<Vec<InodePtr>>);

impl InodePtrManager {
    pub fn new() -> Self {
        let mut v = Vec::new();
        for _ in 0..NINODES {
            v.push(InodePtr::new());
        }
        Self(Mutex::new(v))
    }

    // mark an inode allocated in disk
    // and return an InodePtr with NonePtr
    pub fn alloc_inode(&self, dev: Arc<dyn BlockDevice>, ftype: FileType) -> InodePtr{
        for i in ROOTINO..unsafe {SB.ninodes} {
            let (bno,  off) = addr_of_inode(i);
            let blk = get_buffer_block(bno, dev.clone());
            let mut blk_guard = blk.write().unwrap();
            let mut dinode = blk_guard.read(off as usize, |dinode: &DiskInode| *dinode);
            if dinode.ftype == FileType::Free as u16 {
                dinode.ftype = ftype as u16;
                log_write(&mut blk_guard, off as usize, |diskinode: &mut DiskInode| {
                    *diskinode = dinode;
                });
                return self.get_inode(dev.clone(), i);
            }
        }
        panic!("InodePtrManager::alloc_inode: no free inode");
    }

    pub fn get_inode(&self, dev: Arc<dyn BlockDevice>, inum: u32) -> InodePtr {
        let mut guard = self.0.lock().unwrap();
        let mut empty = 0;
        for (i, inode) in guard.iter().enumerate() {
            if Arc::strong_count(&inode.0) > 1 && inode.0.inum == inum {
                return InodePtr(Arc::clone(&inode.0));
            }
            if empty == 0 && Arc::strong_count(&inode.0) == 1 {
                empty = i + 1;
            }
        }
        if empty == 0 {
            panic!("InodePtrManager::get_inode: no empty inode");
        }
        let i = empty - 1;
        guard[i] = InodePtr(Arc::new(Inode {
            dev: Some(dev.clone()),
            inum,
            dinode: Mutex::new(None),
        }));
        return InodePtr(Arc::clone(&guard[i].0));
    }
}

impl Drop for InodePtr {
    fn drop(&mut self) {
        // lock the table
        let table_guard = unsafe { InodeCache.0.lock().unwrap() };
        // the inode is in table and drop by caller
        // if the table drop it, will not truncate
        if Arc::strong_count(&self.0) == 2 {
            let mut dinode = self.0.dinode.lock().unwrap();
            if dinode.is_some() {
                let dinode = dinode.as_mut().unwrap();
                if dinode.nlink == 0 {
                    // truncate the inode
                    drop(table_guard);
                    Inode::truncate(self.0.dev.as_ref().unwrap().clone(), dinode);
                    // update on disk
                    self.0.modify_disk_inode(|dinode| {
                        dinode.ftype = FileType::Free as u16;
                        dinode.size = 0;
                    });
                }
            }
        }
    }
}

static mut InodeCache: Lazy<InodePtrManager> = Lazy::new(|| InodePtrManager::new());

#[cfg(test)]
mod test {
    use std::{
        fs::{File, OpenOptions},
        sync::{Arc, Mutex},
    };

    #[test]
    fn test_guard_and_ref() {
        let a = Some(Mutex::new(1));
        {
            let mut b = a.as_ref().unwrap().lock().unwrap();
            *b = 2;
        }
        let c = *a.as_ref().unwrap().lock().unwrap();
        assert!(c == 2)
    }

    use env_logger::{Builder, Target};

    use crate::fs::{
        buffer::{get_buffer_block, sync_all},
        filedisk::FileDisk,
        fs::{BLOCK_SIZE, ROOTINO, FileType},
        inode::DirEntry, superblock::SB, log::LOG_MANAGER,
    };

    use super::InodePtrManager;
    #[test]
    fn test_get_inode() {
        let file: File = OpenOptions::new()
            .read(true)
            .write(true)
            .create(false)
            .open("./test.img")
            .unwrap();
        let filedisk = Arc::new(FileDisk::new(file));
        let manager = InodePtrManager::new();
        let mut inode = manager.get_inode(filedisk.clone(), ROOTINO);
        // sb init
        unsafe {SB.init(filedisk.clone())};
        // ls root
        let entries = inode.read_disk_inode(|diskinode| {
            let mut entries = Vec::new();
            for i in 0..super::NDIRECT {
                if diskinode.addrs[i as usize] != 0 {
                    // read entries
                    for j in (0..BLOCK_SIZE).step_by(std::mem::size_of::<DirEntry>()) {
                        let entry = get_buffer_block(diskinode.addrs[i as usize], filedisk.clone())
                            .read()
                            .unwrap()
                            .read(j as usize, |entry: &DirEntry| *entry);
                        if entry.inum != 0 {
                            entries.push(entry);
                        }
                    }
                }
            }
            entries
        });
        // . and ..
        assert!(entries.len() == 2);
        assert!(inode.0.inum == 1);
    }

    #[test]
    fn test_file_create() {
        // log
        Builder::new()
            .target(Target::Stdout)
            .is_test(true)
            .filter_level(log::LevelFilter::Info)
            .init();
        let file: File = OpenOptions::new()
            .read(true)
            .write(true)
            .create(false)
            .open("./test.img")
            .unwrap();
        let filedisk = Arc::new(FileDisk::new(file));
        unsafe {SB.init(filedisk.clone())};
        unsafe {LOG_MANAGER.init(&SB, filedisk.clone())};
        let manager = InodePtrManager::new();
        let inode = manager.alloc_inode(filedisk.clone(), FileType::File);
        inode.0.modify_disk_inode(|diskinode| {
            diskinode.nlink = 1;
            diskinode.size = 0;
        });
        sync_all();
    }
}
