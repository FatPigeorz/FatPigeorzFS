use std::sync::Arc;
use super::superblock::*;
use super::bitmap::*;
use super::inode::*;

// Disk layout:
// [ boot block | super block | log | inode blocks |  bit freemap | data blocks]
pub const SB_BLOCK: u32 = 1;
// Bitmap bits per block
pub const BPB: u32 = BLOCK_SIZE * 8;

pub const FATPIGEORZMAGIC: u32 = 0x14451100;
pub const ROOTINO: u32 = 1;
pub const NDIRECT: u32 = 11;
pub const NAMESIZE: u32 = 14;
pub const NINDIRECT: u32 = BLOCK_SIZE / std::mem::size_of::<u32>() as u32;
pub const MAXFILE: u32 = NDIRECT + NINDIRECT + NINDIRECT * NINDIRECT;

pub const BLOCK_SIZE : u32 = 512;
pub const BLOCK_NUM: u32 = MAXOPBLOCKS * 4;
pub const SHARD_NUM : u32 = 4;


// Maxinum of blocks an FS op can write
pub const MAXOPBLOCKS: u32 = 16;
// Size of log buffer + log header
pub const LOGSIZE: u32 = MAXOPBLOCKS * 3 + 1;

pub const NINODES : u32 = 1024;
// Inodes per block.
pub const IPB : u32 = BLOCK_SIZE / (std::mem::size_of::<Dinode>() as u32);

#[inline]
pub fn block_of_inode(inum: u32, sb: &SuperBlock) -> u32 {
    (inum / IPB) + sb.inodestart
}

pub trait BlockDevice : Send + Sync {
    fn read_block(&self, block_id: u32, buf: &mut [u8]);
    fn write_block(&self, block_id: u32, buf: &[u8]);
}

#[derive(Copy, Clone)]
pub enum FileType {
    None = 0,
    File = 1,
    Dir = 2,
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

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_filetype_as_u32() {
        let filetype = FileType::Dir;
        assert_eq!(filetype as u32, 2);
        let filetype = FileType::File;
        assert_eq!(filetype as u32, 1);
        let filetype = FileType::None;
        assert_eq!(filetype as u32, 0);
    }
}