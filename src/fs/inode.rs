use std::sync::Arc;

use crate::fs::fs::BLOCK_SIZE;

use super::log::log_write;
use super::{
    buffer::get_buffer_block,
    fs::{BlockDevice, BPB, IPB, NAMESIZE, NDIRECT},
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
    pub dev: Arc<dyn BlockDevice>,
    pub inum: u32,
    pub valid: bool,
    pub disk_inode: DiskInode,
}

impl Inode {
    pub fn new(dev: Arc<dyn BlockDevice>, inum: u32) -> Self {
        Self {
            dev,
            inum,
            valid: false,
            disk_inode: DiskInode::default(),
        }
    }
}

impl Inode {
    fn read_disk_inode<V>(&self, f: impl FnOnce(&DiskInode) -> V) -> V {
        let (blk, off) = addr_of_inode(self.inum);
        get_buffer_block(blk, self.dev.clone())
            .read()
            .unwrap()
            .read(off as usize, f)
    }

    fn modify_disk_inode<V>(&self, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        let (blk, off) = addr_of_inode(self.inum);
        log_write(
            &mut get_buffer_block(blk, self.dev.clone()).write().unwrap(),
            off as usize,
            f,
        )
    }
}

#[cfg(test)]
mod test {}
