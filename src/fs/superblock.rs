use std::sync::Arc;

use serde::{Serialize, Deserialize};
use super::fs::{FATPIGEORZMAGIC, BlockDevice, BLOCK_SIZE, SB_BLOCK};

// the super block of filesystem
#[derive(Debug, Default, Clone, Copy, PartialEq)]
#[derive(Serialize, Deserialize)]
pub struct SuperBlock {
    magic: u32,                     // Must be FSMAGIC
    pub size: u32,                  // Size of file system image (blocks)
    pub nblocks: u32,               // Number of data blocks
    pub ninodes: u32,               // Number of inodes
    pub nlog: u32,                  // Number of log blocks
    pub logstart: u32,              // Block number of first log block
    pub inodestart: u32,            // Block number of first inode block
    pub bmapstart: u32,             // Block number of first free map block
}

impl SuperBlock {
    pub fn new() -> Self {
        Self {
            magic: FATPIGEORZMAGIC,
            size: 0,
            nblocks: 0,
            ninodes: 0,
            nlog: 0,
            logstart: 0,
            inodestart: 0,
            bmapstart: 0,
        }
    }

    // Read and init the super block from disk into memory.
    // SAFETY: it should only be called by the first regular process alone.
    pub fn init(&mut self, dev: Arc<dyn BlockDevice>) {
        let mut buf = [0u8; BLOCK_SIZE as usize];
        dev.read_block(SB_BLOCK, &mut buf);
        let sb: &SuperBlock = unsafe { std::mem::transmute(&buf) };
        *self = *sb;
    }
}