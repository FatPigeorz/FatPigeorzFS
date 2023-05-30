use crate::fs::fs::*;
use crate::fs::inode::*;
use crate::fs::log::*;
use crate::fs::superblock::*;
use env_logger::{Builder, Target};
use log::info;
use std::{
    fs::{File, OpenOptions},
    io::*,
    os::unix::prelude::FileExt,
    path::PathBuf,
};

fn write_block(file: &mut File, block_id: u32, buf: &[u8]) {
    file.write_at(buf, (block_id * BLOCK_SIZE) as u64)
        .expect("write failed!");
}

fn read_block(file: &mut File, block_id: u32, buf: &mut [u8]) {
    file.read_exact_at(buf, (block_id * BLOCK_SIZE) as u64)
        .expect("read failed!");
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
    let fs_size = size / BLOCK_SIZE;
    let nbitmap = fs_size / (BLOCK_SIZE * 8);
    let ninodeblocks = NINODES / IPB;
    let nlog = LOGSIZE;
    let nmeta = 2 + nlog + ninodeblocks + nbitmap;

    // superblock
    let mut sb = SuperBlock::new();
    sb.size = fs_size as u32;
    sb.nblocks = (fs_size - nmeta) as u32;
    sb.ninodes = NINODES;
    sb.nlog = nlog;
    // 0 is reserved for root inode
    // 1 is reserved for superblock
    sb.logstart = 2;
    sb.inodestart = 2 + nlog;
    sb.bmapstart = 2 + nlog + ninodeblocks;

    // log the metadata
    info!(
        "fs_size: {}, nbitmap: {}, ninodeblocks: {}, nlog: {}, nmeta: {}",
        fs_size, nbitmap, ninodeblocks, nlog, nmeta
    );

    // list  size
    info!("disk datastructure size:");
    info!("SuperBlock: {}", std::mem::size_of::<SuperBlock>());
    info!("LogHeader: {}", std::mem::size_of::<LogHeader>());
    info!("Dinode: {}", std::mem::size_of::<DiskInode>());
    info!("DirEntry: {}", std::mem::size_of::<DirEntry>());
    info!("File: {}", std::mem::size_of::<File>());

    // list the disk layout
    info!("Disk layout:");
    info!("boot block: 0");
    info!("super block: 1");
    info!("log header: 2");
    info!("log: 3 - {}", 2 + nlog - 1);
    info!(
        "inode blocks: {} - {}",
        2 + nlog,
        2 + nlog + ninodeblocks - 1
    );
    info!(
        "free bit map: {} - {}",
        2 + nlog + ninodeblocks,
        2 + nlog + ninodeblocks + nbitmap - 1
    );
    info!(
        "data blocks: {} - {}",
        2 + nlog + ninodeblocks + nbitmap,
        fs_size - 1
    );

    // serialize sb
    let mut buf = [0; 512];
    unsafe {
        std::ptr::copy(
            &sb as *const SuperBlock as *const u8,
            buf.as_mut_ptr(),
            std::mem::size_of::<SuperBlock>(),
        );
    }
    info!("write superblock at block {}", 1);
    write_block(&mut file, SB_BLOCK, &buf);

    // the first free block that we can allocate
    let mut freeblock = nmeta;
    let mut freeino = ROOTINO;

    // write root inode
    let rootino = ialloc(&mut file, &sb, FileType::Dir, &mut freeino);
    assert_eq!(rootino, ROOTINO);

    let mut de = DirEntry::default();
    de.inum = rootino;
    // de.name = ".".to_string();
    nameassign(&mut de.name, &".".to_string());
    let buf = unsafe { std::mem::transmute::<DirEntry, [u8; std::mem::size_of::<DirEntry>()]>(de) };
    iappend(&mut file, rootino, &sb, &buf, &mut freeblock);

    let mut de = DirEntry::default();
    de.inum = rootino;
    nameassign(&mut de.name, &"..".to_string());
    let buf = unsafe { std::mem::transmute::<DirEntry, [u8; std::mem::size_of::<DirEntry>()]>(de) };
    iappend(&mut file, rootino, &sb, &buf, &mut freeblock);

    // fix size of root
    let dinode = rinode(&mut file, &sb, rootino);
    winode(&mut file, &sb, rootino, dinode);

    balloc(&mut file, &sb, freeblock);
}

fn balloc(file: &mut File, sb: &SuperBlock, used: u32) {
    let mut buf = vec![0; BLOCK_SIZE as usize];
    info!("balloc: first {} blocks have been allocated", used);
    assert!(used < BLOCK_SIZE * 8);
    for i in 0..used {
        buf[i as usize / 8] |= 1 << (i % 8);
    }
    info!("balloc: write bitmap block at block {}", sb.bmapstart);
    write_block(file, sb.bmapstart, &buf);
}

fn ialloc(file: &mut File, sb: &SuperBlock, filetype: FileType, freeinode: &mut u32) -> u32 {
    let inum = *freeinode;
    *freeinode += 1;

    let mut dinode = DiskInode::default();
    dinode.ftype = filetype as u16;
    dinode.nlink = 1;
    dinode.size = 0;
    // write
    winode(file, sb, inum, dinode);
    inum
}

// append data to inode
fn iappend(file: &mut File, inum: u32, sb: &SuperBlock, data: &[u8], freeblock: &mut u32) {
    let mut dinode = rinode(file, sb, inum);
    let mut indirect = [0u32; NINDIRECT as usize];
    let mut off = dinode.size; // the offset of the file
    let mut n = data.len() as u32;
    let mut data_ptr = data;
    let mut dst_block;
    info!(
        "iappend: inum: {}, size: {}, off: {}, n: {}",
        inum, dinode.size, off, n
    );
    while n > 0 {
        let fbn = off / BLOCK_SIZE;
        assert!(fbn < MAXFILE as u32);
        // read block
        if fbn < NDIRECT {
            if dinode.addrs[fbn as usize] == 0 {
                // allocate a new block
                dinode.addrs[fbn as usize] = *freeblock;
                *freeblock += 1;
            }
            dst_block = dinode.addrs[fbn as usize];
        } else {
            // read the indirect block
            if dinode.addrs[NDIRECT as usize] == 0 {
                // allocate the indirect inode
                dinode.addrs[NDIRECT as usize] = *freeblock;
                *freeblock += 1;
            }
            // read to indirect
            // cast the indirect to [u8, BLOCK_SIZE]
            let mut buf = unsafe {
                std::slice::from_raw_parts_mut(
                    indirect.as_mut_ptr() as *mut u8,
                    BLOCK_SIZE as usize,
                )
            };
            read_block(file, dinode.addrs[NDIRECT as usize], &mut buf);
            if indirect[fbn as usize - NDIRECT as usize] == 0 {
                indirect[fbn as usize - NDIRECT as usize] = *freeblock;
                *freeblock += 1;
                // write indirect
                let buf = unsafe {
                    std::slice::from_raw_parts_mut(
                        indirect.as_mut_ptr() as *mut u8,
                        BLOCK_SIZE as usize,
                    )
                };
                write_block(file, dinode.addrs[NDIRECT as usize], buf)
            }
            dst_block = indirect[fbn as usize - NDIRECT as usize];
        }
        // write the data to block
        let bytes = std::cmp::min(n as u32, (fbn + 1) * BLOCK_SIZE - off);
        // read dst block
        let mut buf = [0; BLOCK_SIZE as usize];
        info!("iappend: read block {} to write", dst_block);
        read_block(file, dst_block, &mut buf);
        // copy data to dst block at offset
        buf[(off % BLOCK_SIZE) as usize..(off % BLOCK_SIZE + bytes) as usize]
            .copy_from_slice(&data_ptr[..bytes as usize]);
        info!("iappend: write block {}", dst_block);
        write_block(file, dst_block, &buf);
        n -= bytes;
        off += bytes;
        data_ptr = &data_ptr[bytes as usize..];
    }
    dinode.size = off;
    winode(file, sb, inum, dinode);
}

fn block_of_inode(inum: u32, sb: &SuperBlock) -> u32 {
    sb.inodestart + inum / IPB
}

fn rinode(file: &mut File, sb: &SuperBlock, inum: u32) -> DiskInode {
    let mut buf = [0; BLOCK_SIZE as usize];
    info!(
        "rinode: read inode block at block {}",
        block_of_inode(inum, sb)
    );
    read_block(file, block_of_inode(inum, sb), &mut buf);
    // use transmute instead
    unsafe {
        let ptr = buf.as_ptr() as *const DiskInode;
        return *ptr.add(inum as usize % IPB as usize);
    }
}

fn winode(file: &mut File, sb: &SuperBlock, inum: u32, dinode: DiskInode) {
    let mut buf = [0; BLOCK_SIZE as usize];
    unsafe {
        let ptr = buf.as_mut_ptr() as *mut DiskInode;
        ptr.add(inum as usize % IPB as usize).write(dinode);
    }
    info!(
        "winode: write inode block at block {}",
        block_of_inode(inum, sb)
    );
    write_block(file, block_of_inode(inum, sb), &buf);
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_size() {
        println!("DirEntry:  {}", std::mem::size_of::<DirEntry>());
        println!("DiskInode: {}", std::mem::size_of::<DiskInode>());
    }

    #[test]
    fn test_mkfs() {
        mkfs("./test.img".into(), 512 * 512 * 8);
    }
}
