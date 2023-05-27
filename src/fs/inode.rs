use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;

use crate::fs::fs::BLOCK_SIZE;

use super::fs::NINODES;
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
    pub dev: u32,                            // Device number, always 0
    pub ftype: u16,                          // File type
    pub nlink: u16,                          // Number of links to file
    pub size: u32,                           // Size of file (bytes)
    pub blocks: [u32; NDIRECT as usize + 2], // Pointers to blocks
}

// directory contains a sequence of entry
#[repr(C)]
#[derive(Debug, Default)]
#[derive(Clone, Copy)]
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
    (inum / IPB + unsafe { SB.inodestart }, inum % IPB)
}

// get the block containing the bitmap
fn block_of_bitmap(block: u32) -> u32 {
    block / BPB + unsafe { SB.bmapstart }
}

pub struct Inode {
    pub dev: Option<Arc<dyn BlockDevice>>,
    pub inum: u32,
    pub valid: bool,
}

impl Inode {
    pub fn new() -> Self {
        Self {
            dev: None,
            inum: 0,
            valid: false,
        }
    }

    pub fn init(&mut self, dev: Arc<dyn BlockDevice>, inum: u32) {
        self.dev = Some(dev);
        self.inum = inum;
        self.valid = false;
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

    pub fn entry_list(&self) -> Option<Vec<DirEntry>> {
        self.read_disk_inode(|diskinode| {
            if diskinode.ftype != FileType::Dir as u16 {
                return None;
            } 
            let mut entries = Vec::new();
            for blk in diskinode.blocks.iter() {
                if *blk == 0 {
                    continue;
                }
                for i in (0..(BLOCK_SIZE as usize)).step_by(std::mem::size_of::<DirEntry>()) {
                    let entry = get_buffer_block(*blk, self.dev.as_ref().unwrap().clone())
                        .read()
                        .unwrap()
                        .read(
                            i as usize,
                            |entry: &DirEntry| *entry
                        );
                    if entry.inum == 0 {
                        continue;
                    }
                    entries.push(entry);
                }
            }
            Some(entries)
        })
    }
}

pub struct InodeTable (Vec<Arc<Mutex<Inode>>>);

impl InodeTable {
    pub fn new() -> Self {
        let mut v = Vec::new();
        for _ in 0..NINODES {
            v.push(Arc::new(Mutex::new(Inode::new())));
        }
        Self(v)
    }

    pub fn get_inode(&self, dev: Arc<dyn BlockDevice>, inum: u32) -> Arc<Mutex<Inode>> {
        let mut inode = self.0[inum as usize].lock().unwrap();
        if inode.valid {
            return self.0[inum as usize].clone();
        }
        inode.init(dev, inum);
        inode.valid = true;
        self.0[inum as usize].clone()
    }
}


pub static mut INODETABLE: Lazy<InodeTable> = Lazy::new(|| InodeTable::new());

#[cfg(test)]
mod test {}
