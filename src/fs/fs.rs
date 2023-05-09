// Disk layout:
// [ boot block | super block | log | inode blocks | free bit map | data blocks]

const FATPIGEORZMAGIC: u32 = 0x10203040;
const ROOTINO: u32 = 1;
const BSIZE: u32 = 1024;

const NDIRECT: u32 = 11;
const MAXFILE: u32 = NDIRECT + NDIRECT * NDIRECT;

// the super block of filesystem
#[derive(Debug, Default)]
pub struct SuperBlock {
    pub magic: u32,                 // Must be FSMAGIC
    pub size: u32,                  // Size of file system image (blocks)
    pub nblocks: u32,               // Number of data blocks
    pub ninodes: u32,               // Number of inodes
    pub nlog: u32,                  // Number of log blocks
    pub logstart: u32,              // Block number of first log block
    pub inodestart: u32,            // Block number of first inode block
    pub bmapstart: u32,             // Block number of first free map block
}

// INode
#[derive(Debug, Default)]
pub struct Inode {
    pub dev: u32,                            // Device number
    pub inum: u32,                           // Inode number
    pub valid: u32,                          // Valid?
    pub size: u32,                           // Size of file (bytes)
    pub nlink: u32,                          // Number of links to file
    pub blocks: [u32; NDIRECT as usize + 2], // Pointers to blocks
}

// the bitmap
#[derive(Debug, Default)]
pub struct Bitmap {
    pub data: Vec<u8>,
}

// the file system
#[derive(Debug, Default)]
pub struct FileSystem {
    pub superblock: Option<SuperBlock>,
    pub bitmap: Option<Bitmap>,
    pub inode: Option<Vec<Inode>>,
}