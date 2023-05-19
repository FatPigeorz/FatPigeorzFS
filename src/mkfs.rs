use std::{path::PathBuf, fs::{OpenOptions, File}, io::*, os::unix::prelude::FileExt};
use crate::fs::fs::*;
use env_logger::{Builder, Target};
use log::{debug, error, log_enabled, info, Level};

fn write_block(file: &mut File, block_id: u32, buf: &[u8]) {
    file.write_at(buf, (block_id * BLOCK_SIZE) as u64).expect("write failed!");
}

fn read_block(file: &mut File, block_id: u32, buf: &mut [u8]) {
    file.read_exact_at(buf, (block_id * BLOCK_SIZE) as u64).expect("write failed!");
}

// Disk layout:
// [ boot block | sb block | log | inode blocks | free bit map | data blocks ]
pub fn mkfs(path: PathBuf, size: u32) {
    Builder::new()
        .target(Target::Stdout)
        .is_test(true)
        .filter_level(log::LevelFilter::Info)
        .init();

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .unwrap();
    file.set_len(size as u64).unwrap();
    let buf = vec![0; size as usize];
    file.write_all(buf.as_ref()).unwrap();

    // size must be multiple of BLOCK_SIZE
    assert_eq!(size % BLOCK_SIZE, 0);

    // metadata 
    let fs_size= size / BLOCK_SIZE;
    let nbitmap= fs_size / (BLOCK_SIZE * 8) + 1;
    let ninodeblocks = NINODES / IPB + 1;
    let nlog= LOGSIZE;
    let nmeta = 2 + nlog + ninodeblocks + nbitmap;


    // superblock
    let mut sb = SuperBlock::new();
    sb.size = fs_size as u32;
    sb.nblocks = (fs_size - nmeta) as u32;
    sb.nlog = nlog;
    // 0 is reserved for root inode
    // 1 is reserved for superblock
    sb.logstart = 2;
    sb.inodestart = 2 + nlog;
    sb.bmapstart = 2 + nlog + ninodeblocks;

    // log the metadata
    info!("fs_size: {}, nbitmap: {}, ninodeblocks: {}, nlog: {}, nmeta: {}", fs_size, nbitmap, ninodeblocks, nlog, nmeta);

    // serialize sb
    let mut buf = [0; 512];
    let sb_bytes = bincode::serialize(&sb).unwrap();
    buf[..sb_bytes.len()].copy_from_slice(&sb_bytes);
    write_block(&mut file, ROOTINO, &buf);

    // serialize bitmap
}

#[cfg(test)]
mod test {
    use std::{fs::{OpenOptions, File}, io::{Write}};
    use crate::mkfs::{write_block, read_block};
    use super::*;

    #[test]
    fn test_sb_serialize() {
        // make file
        let mut file: File= OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open("./test.img")
            .unwrap();
        file.set_len(1024 * 1024).unwrap();
        file.write_all(&[0u8; 1024 * 1024]).unwrap();

        let sb = super::SuperBlock::new();
        // serialize
        // write sb to block 0
        let mut buf = [0; 512];
        let sb_bytes = bincode::serialize(&sb).unwrap();
        buf[..sb_bytes.len()].copy_from_slice(&sb_bytes);
        write_block(&mut file, 1, &buf);
        
        // deserialize
        read_block(&mut file, 1, &mut buf);
        let new_sb: super::SuperBlock = bincode::deserialize(&buf).unwrap();
        assert_eq!(sb, new_sb);
    }
    
    #[test]
    fn test_mkfs() {
        mkfs("./test.img".into(), 1024 * 1024);
    }
}