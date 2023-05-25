use super::buffer::get_buffer_block;
use super::buffer::BufferBlock;
use super::file::*;
use super::inode::*;
use super::log::LOG_MANAGER;
use super::superblock::*;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;

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

pub const BLOCK_SIZE: u32 = 512;
pub const BLOCK_NUM: u32 = MAXOPBLOCKS * 4;
pub const SHARD_NUM: u32 = 4;

// Maxinum of blocks an FS op can write
pub const MAXOPBLOCKS: u32 = 16;
// Size of log buffer + log header
pub const LOGSIZE: u32 = MAXOPBLOCKS * 3 + 1;

pub const NINODES: u32 = 1024;
// Inodes per block.
pub const IPB: u32 = BLOCK_SIZE / (std::mem::size_of::<Dinode>() as u32);

pub const NFILE: u32 = 100;
pub const NOFILE: u32 = 16;

pub trait BlockDevice: Send + Sync {
    fn read_block(&self, block_id: u32, buf: &mut [u8]);
    fn write_block(&self, block_id: u32, buf: &[u8]);
}

// the file system
pub struct FileSystem {
    pub device: Arc<dyn BlockDevice>,
    pub sb: Option<Arc<RwLock<BufferBlock>>>,
    pub lh: Option<Arc<RwLock<BufferBlock>>>,
    pub bitmap: Option<Arc<RwLock<BufferBlock>>>,
}

impl FileSystem {
    pub fn new(device: Arc<dyn BlockDevice>) -> Self {
        let mut fs = Self {
            device: device,
            sb: None,
            lh: None,
            bitmap: None,
        };
        fs.init();
        fs
    }

    fn init(&mut self) {
        unsafe {
            SB.init(self.device.clone());
        }
        self.sb = Some(get_buffer_block(SB_BLOCK, self.device.clone()));
        unsafe { LOG_MANAGER.init(&SB, self.device.clone()) };
        self.lh = Some(get_buffer_block(
            unsafe { SB.logstart },
            self.device.clone(),
        ));
        self.bitmap = Some(get_buffer_block(
            unsafe { SB.bmapstart },
            self.device.clone(),
        ));
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

    #[test]
    fn test_init() {}
}
