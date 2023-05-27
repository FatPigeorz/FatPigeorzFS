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
    pub nlog: u32,       // Number of log blocks
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
        let sb_block = get_buffer_block(SB_BLOCK, dev.clone());
        let sb_guard = sb_block.read().unwrap();
        let sb: &SuperBlock = sb_guard.as_ref(0);
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
    }
}

pub static mut SB: Lazy<SuperBlock> = Lazy::new(|| SuperBlock::new());

#[cfg(test)]
mod test {
    use std::fs::{File, OpenOptions};

    use crate::fs::filedisk::FileDisk;

    use super::super::fs::*;
    use super::*;
    #[test]
    fn test_init() {
        let file: File = OpenOptions::new()
            .read(true)
            .write(false)
            .create(false)
            .open("./test.img")
            .unwrap();
        let dev = Arc::new(FileDisk::new(file));
        unsafe {
            SB.init(dev.clone());
        }
        // boot block: 0
        // super block: 1
        // log header: 2
        // log: 3 - 50
        // inode blocks: 51 - 178
        // free bit map: 179 - 179
        // data blocks: 180 - 2047

        assert_eq!(unsafe { SB.size }, 2048);
        assert_eq!(unsafe { SB.nblocks }, 2048 - 180);
        assert_eq!(unsafe { SB.ninodes }, NINODES);
        assert_eq!(unsafe { SB.nlog }, LOGSIZE);
        assert_eq!(unsafe { SB.logstart }, 2);
        assert_eq!(unsafe { SB.inodestart }, 51);
        assert_eq!(unsafe { SB.bmapstart }, 179);
        print!("superblock: {:?}\n", unsafe { *SB });
    }
}
