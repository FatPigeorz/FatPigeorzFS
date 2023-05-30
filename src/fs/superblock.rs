use std::sync::Arc;

use super::buffer::get_buffer_block;
use super::fs::{BlockDevice, FATPIGEORZMAGIC, SB_BLOCK};
use once_cell::sync::Lazy;

// the super block of filesystem
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct SuperBlock {
    magic: u32,          // Must be FSMAGIC
    pub size: u32,       // Size of file system image (blocks)
    pub nblocks: u32,    // Number of data blocks
    pub ninodes: u32,    // Number of inodes
    pub nlog: u32,       // Numbe of log blocks
    pub logstart: u32,   // Block number of first log block
    pub inodestart: u32, // Block number of first inode block
    pub bmapstart: u32,  // Block number of first free map block
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

    pub fn init(&mut self, dev: Arc<dyn BlockDevice>) {
        get_buffer_block(SB_BLOCK, dev.clone())
            .read()
            .unwrap()
            .read(0, |sb: &SuperBlock| {
                // assert magic eq and panic
                if self.magic != FATPIGEORZMAGIC {
                    panic!("SuperBlock::init: invalid magic number");
                }
                self.size = sb.size;
                self.nblocks = sb.nblocks;
                self.ninodes = sb.ninodes;
                self.nlog = sb.nlog;
                self.logstart = sb.logstart;
                self.inodestart = sb.inodestart;
                self.bmapstart = sb.bmapstart;
            });
    }
}

pub static mut SB: Lazy<SuperBlock> = Lazy::new(|| SuperBlock::new());
