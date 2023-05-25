// Disk layout:
// [ boot block | super block | log | inode blocks |  bit freemap | data blocks]
pub const SB_BLOCK: u32 = 1;
// Bitmap bits per block
pub const BPB: u32 = BLOCK_SIZE * 8;

pub const FATPIGEORZMAGIC: u32 = 0x14451100;
pub const ROOTINO: u32 = 1;
pub const NDIRECT: u32 = 11;
pub const NAMESIZE: u32 = 10;
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
use super::inode::Dinode;
pub const IPB: u32 = BLOCK_SIZE / (std::mem::size_of::<Dinode>() as u32);

pub const NFILE: u32 = 100;
pub const NOFILE: u32 = 16;

pub trait BlockDevice: Send + Sync {
    fn read_block(&self, block_id: u32, buf: &mut [u8]);
    fn write_block(&self, block_id: u32, buf: &[u8]);
}