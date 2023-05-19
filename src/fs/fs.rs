use std::sync::Arc;
use serde::{Serialize, Deserialize};

// Disk layout:
// [ boot block | super block | log | inode blocks | free bit map | data blocks]
pub const SB_BLOCK: u32 = 1;

pub const FATPIGEORZMAGIC: u32 = 0x14451100;
pub const ROOTINO: u32 = 1;
pub const NDIRECT: u32 = 11;
pub const NINDIRECT: u32 = BLOCK_SIZE / std::mem::size_of::<u32>() as u32;
pub const MAXFILE: u32 = NDIRECT + NINDIRECT + NINDIRECT * NINDIRECT;

pub const BLOCK_SIZE : u32 = 512;
pub const BLOCK_NUM: u32 = MAXOPBLOCKS * 4;
pub const SHARD_NUM : u32 = 4;


// Maxinum of blocks an FS op can write
pub const MAXOPBLOCKS: u32 = 16;
// Size of log buffer + log header
pub const LOGSIZE: u32 = MAXOPBLOCKS * 3 + 1;


pub const NINODES : u32 = 1000;
// Inodes per block.
pub const IPB : u32 = BLOCK_SIZE / (std::mem::size_of::<Dinode>() as u32);

pub trait BlockDevice : Send + Sync {
    fn read_block(&self, block_id: u32, buf: &mut [u8]);
    fn write_block(&self, block_id: u32, buf: &[u8]);
}

// the file disk device

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

    pub fn init(&mut self, dev: Arc<dyn BlockDevice>) {
        let mut buf = [0u8; BLOCK_SIZE as usize];
        dev.read_block(SB_BLOCK, &mut buf);
        let sb: &SuperBlock = unsafe { std::mem::transmute(&buf) };
        *self = *sb;
    }
}

enum FileType {
    None,
    Dir,
    File,
}

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

// INode in disk
#[derive(Debug, Default, Clone, Copy)]
pub struct Dinode {
    pub dev: u32,                            // Device number, always 0
    pub inum: u32,                           // Inode number
    pub valid: u32,                          // Valid?
    pub size: u32,                           // Size of file (bytes)
    pub nlink: u32,                          // Number of links to file
    pub blocks: [u32; NDIRECT as usize + 2], // Pointers to blocks
}

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

    fn init(&mut self, block_id: u32, dev: Arc<dyn BlockDevice>) {
        let buf = &mut [0u8; BLOCK_SIZE as usize];
        dev.read_block(block_id, buf);
    }
}

// the file system
pub struct FileSystem {
    pub device: Arc<dyn BlockDevice>,
    pub superblock: Option<SuperBlock>,
    pub bitmap: Option<Bitmap>,
    pub inode: Option<Vec<Inode>>,
}

impl FileSystem {
    pub fn new(device: Arc<dyn BlockDevice>) -> Self {
        let mut fs = Self {
            device: device,
            superblock: None,
            bitmap: None,
            inode: None,
        };
        fs.init();
        fs
    }

    fn init(&mut self) {
        // init super block
        self.superblock.unwrap().init(self.device.clone());
        let sb = &self.superblock.unwrap();

        // init inodes
        let mut inodes = Vec::new();
        for i in 0..sb.ninodes {
            let mut buf = [0u8; BLOCK_SIZE as usize];
            self.device.read_block(sb.inodestart + i / 8, &mut buf);
            let inode: &Inode = unsafe { std::mem::transmute(&buf) };
            inodes.push(inode);
        }
    }
}