use std::sync::Arc;

use super::log::*;
use super::buffer::*;
use super::fs::{BLOCK_SIZE, BLOCK_NUM, BPB, IPB, BlockDevice};
use super::superblock::SuperBlock;

const BLOCK_BITS: u32 = BLOCK_NUM * 8;

// the bitmap
#[derive(Debug, Clone, Copy)]
pub struct Bitmap {
    pub data: [u8; (BLOCK_BITS / 8) as usize],
}

impl Bitmap {
    fn new() -> Self {
        Self {
            data: [0; (BLOCK_SIZE as usize) / 8],
        }
    }

    // TODO: the bitmap should be initialized with the superblock
    fn init(&mut self, sb: &SuperBlock, dev: Arc<dyn BlockDevice>) {
        let buf = &mut [0u8; BLOCK_SIZE as usize];
        let nbitmap = (sb.nblocks + BPB - 1) / BPB;
        for i in 0..sb.size {
            dev.read_block(sb.bmapstart + i, buf);
            self.data[i as usize] = buf[0];
        }
    }
}


// Given an inode number. 
// Calculate the offset index of this inode inside the block. 
#[inline]
fn locate_inode(ino: u32) -> usize {
    (ino % IPB) as usize
}

// Allocate a zeroed disk block
pub fn balloc(dev: Arc<dyn BlockDevice>) -> u32 {
    0
}
