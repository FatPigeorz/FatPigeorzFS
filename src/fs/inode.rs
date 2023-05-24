use std::sync::Arc;
use serde::{Serialize, Deserialize};

use super::fs::{FileType, NDIRECT, BlockDevice};

// INode in memory
pub struct Inode {
    dev: Arc<dyn BlockDevice>,           // Device
    ftype: FileType,                     // FileType
    inum: u32,                           // Inode number
    valid: u32,                          // Valid?
    size: u32,                           // Size of file (bytes)
    nlink: u32,                          // Number of links to file
    blocks: [u32; NDIRECT as usize + 2], // Pointers to blocks
}

// INode in disk, the size is 64!
#[derive(Debug, Default, PartialEq)]
#[derive(Serialize, Deserialize)]
pub struct Dinode {
    pub dev: u32,                            // Device number, always 0
    pub ftype: u16,                          // File type
    pub nlink: u16,                          // Number of links to file
    pub size: u32,                           // Size of file (bytes)
    pub blocks: [u32; NDIRECT as usize + 2], // Pointers to blocks
}

// directory contains a sequence of entry
#[derive(Debug, Default)]
#[derive(Serialize, Deserialize)]
pub struct DirEntry {
    pub inum: u32,
    pub name: String,
}

#[cfg(test)]
mod test {
    use crate::fs::fs::BLOCK_SIZE;
    use super::*;
    #[test]
    fn test_dinode_size() {
        println!("size of Dinode: {}", std::mem::size_of::<Dinode>());
        assert_eq!(BLOCK_SIZE as usize % std::mem::size_of::<Dinode>(), 0);
    }
}