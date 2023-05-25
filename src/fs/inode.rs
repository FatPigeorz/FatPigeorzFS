use env_logger::{Builder, Target};
use log::info;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::RwLockWriteGuard;
use std::{
    fs::File,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};

use super::buffer::BufferBlock;
use super::file::FileType;
use super::fs::MAXFILE;
use super::{
    buffer::get_buffer_block,
    fs::{BlockDevice, BLOCK_SIZE, BPB, IPB, NDIRECT, NINDIRECT, NINODES, ROOTINO},
    log::log_write,
    superblock::SB,
};

// INode in memory
pub struct Inode {
    dev: Arc<dyn BlockDevice>, // Device
    inum: u32,                 // Inode number
    valid: bool,               // Valid?

    ftype: FileType,                     // FileType
    nlink: u16,                          // Number of links to file
    size: u32,                           // Size of file (bytes)
    blocks: [u32; NDIRECT as usize + 2], // Pointers to blocks
}

pub struct ICache {
    inodes: Arc<Mutex<Vec<Option<Arc<Mutex<Inode>>>>>>,
}

fn block_of_inode(inum: u32) -> u32 {
    (inum / IPB) + unsafe { SB.inodestart }
}

fn block_of_bitmap(b: u32) -> u32 {
    (b / BPB) + unsafe { SB.bmapstart }
}

fn bzero(dev: Arc<dyn BlockDevice>, bp: &mut RwLockWriteGuard<BufferBlock>) {
    let buf = bp.as_mut::<[u8; BLOCK_SIZE as usize]>(0);
    buf.fill(0);
}

fn balloc(dev: Arc<dyn BlockDevice>) -> u32 {
    for b in (0..unsafe { SB.size }).step_by(BPB as usize) {
        let block = get_buffer_block(block_of_bitmap(b), dev.clone());
        // get the block of bitmap
        let mut bp = block.write().unwrap();
        let buf = &mut *(bp.as_mut::<[u8; BLOCK_SIZE as usize]>(0));
        let mut find = 0;
        for bi in 0..BPB {
            let mask = 1 << (bi % 8);
            if buf[(bi / 8) as usize] & mask == 0 {
                buf[(bi / 8) as usize] |= mask;
                find = bi + 1;
            }
        }
        if find > 0 {
            bzero(dev, &mut bp);
            log_write(&bp);
            return b + find - 1;
        }
    }
    panic!("balloc: out of blocks");
}

fn bfree(dev: Arc<dyn BlockDevice>, b: u32) {
    let block = get_buffer_block(block_of_bitmap(b), dev.clone());
    let mut bp = block.write().unwrap();
    let buf = bp.as_mut::<[u8; BLOCK_SIZE as usize]>(0);
    let bi = b % BPB;
    let mask = 1 << (bi % 8);
    assert!(buf[(bi / 8) as usize] & mask != 0);
    buf[(bi / 8) as usize] &= !mask;
    log_write(&bp);
}
// Inode content
//
// The content (data) associated with each inode is stored
// in blocks on the disk. The first NDIRECT block numbers
// are listed in ip->addrs[].  the next nindirect blocks are
// listed in block ip->addrs[ndirect]. the next double-indirect blocks are
// listed in block ip->addrs[ndirect + 1].

// Return the disk block address of the nth block in inode ip.
// If there is no such block, bmap allocates one.
pub fn bmap(inode: &mut MutexGuard<Inode>, bn: u32) -> Result<u32, String> {
    let mut bn = bn;
    if bn < NDIRECT {
        if inode.blocks[bn as usize] == 0 {
            inode.blocks[bn as usize] = balloc(inode.dev.clone());
        }
        return Ok(inode.blocks[bn as usize]);
    }
    bn -= NDIRECT;
    if bn < NINDIRECT {
        if inode.blocks[NDIRECT as usize] == 0 {
            inode.blocks[NDIRECT as usize] = balloc(inode.dev.clone());
        }
        let block = get_buffer_block(inode.blocks[NDIRECT as usize], inode.dev.clone());
        let mut bp = block.write().unwrap();
        let buf = bp.as_mut::<[u32; NINDIRECT as usize]>(0);
        if buf[bn as usize] == 0 {
            buf[bn as usize] = balloc(inode.dev.clone());
        }
        return Ok(buf[bn as usize]);
    }
    bn -= NINDIRECT;
    if bn < NINDIRECT * NINDIRECT {
        if inode.blocks[(NDIRECT + 1) as usize] == 0 {
            inode.blocks[(NDIRECT + 1) as usize] = balloc(inode.dev.clone());
        }
        let block = get_buffer_block(inode.blocks[(NDIRECT + 1) as usize], inode.dev.clone());
        let mut bp: std::sync::RwLockWriteGuard<crate::fs::buffer::BufferBlock> =
            block.write().unwrap();
        let buf = bp.as_mut::<[u32; NINDIRECT as usize]>(0);
        let block = bn / NINDIRECT;
        if buf[block as usize] == 0 {
            buf[block as usize] = balloc(inode.dev.clone());
        }
        let block = get_buffer_block(buf[block as usize], inode.dev.clone());
        let mut bp = block.write().unwrap();
        let buf = bp.as_mut::<[u32; NINDIRECT as usize]>(0);
        if buf[bn as usize] == 0 {
            buf[bn as usize] = balloc(inode.dev.clone());
        }
        return Ok(buf[bn as usize]);
    }
    Err("bmap: out of range".to_string())
}

impl ICache {
    pub fn new() -> Self {
        let mut vec_inodes = Vec::with_capacity(NINODES as usize);
        for _ in 0..NINODES {
            vec_inodes.push(None);
        }
        Self {
            inodes: Arc::new(Mutex::new(vec_inodes)),
        }
    }

    // Allocate an inode on device dev.
    // Mark it as allocated by  giving it type type.
    // Returns an unlocked but allocated and referenced inode,
    // or panic if there is no free inode.
    fn ialloc(&self, dev: Arc<dyn BlockDevice>, ftype: FileType) -> Option<Arc<Mutex<Inode>>> {
        for i in 1..NINODES {
            let block = get_buffer_block(block_of_inode(i), dev.clone());
            let mut bp = block.write().unwrap();
            let buf = bp.as_mut::<[u8; std::mem::size_of::<Dinode>()]>(
                i as usize * std::mem::size_of::<Dinode>(),
            );
            let mut dinode: Dinode = bincode::deserialize(buf).unwrap();
            // let mut dinode: Dinode = bincode::deserialize(buf).unwrap();
            if dinode.ftype == 0 {
                dinode = Dinode {
                    ftype: ftype as u16,
                    ..Default::default()
                };
                let bytes = bincode::serialize(&dinode).unwrap();
                buf[..bytes.len()].copy_from_slice(&bytes);
                log_write(&bp);
                return self.iget(dev, i);
            }
        }
        panic!("ialloc: no inodes");
    }

    // Find the inode with number inum on device dev
    // and return the in-memory copy. Does not lock
    // the inode and does not read it from disk.
    fn iget(&self, dev: Arc<dyn BlockDevice>, inum: u32) -> Option<Arc<Mutex<Inode>>> {
        let mut guard = self.inodes.lock().unwrap();

        let mut empty = 0;
        for i in 1..NINODES {
            // if in icache
            if let Some(inode) = &guard[i as usize] {
                let mut guard = inode.lock().unwrap();
                if guard.inum == inum {
                    guard.valid = true;
                    return Some((*inode).clone());
                }
            } else if empty == 0 {
                empty = i;
                // can't break
            }
        }
        if empty == 0 {
            panic!("iget: no inodes");
        }
        assert!(empty < NINODES);
        assert!(guard[empty as usize].is_none());

        let inode = Arc::new(Mutex::new(Inode {
            dev: dev.clone(),
            ftype: FileType::None,
            inum: inum,
            valid: false,
            size: 0,
            nlink: 0,
            blocks: [0; NDIRECT as usize + 2],
        }));
        guard[empty as usize] = Some(inode.clone());
        Some(inode)
    }

    fn iupdate(&mut self, inode: &MutexGuard<Inode>) {
        let block = get_buffer_block(block_of_inode(inode.inum), inode.dev.clone());
        let mut bp = block.write().unwrap();
        let offset = (inode.inum % IPB) as usize;
        let buf =
            &mut *(bp.as_mut::<[u8; BLOCK_SIZE as usize]>(offset * std::mem::size_of::<Dinode>()));
        let mut dinode: Dinode = bincode::deserialize(buf).unwrap();
        dinode.ftype = inode.ftype as u16;
        dinode.nlink = inode.nlink;
        dinode.size = inode.size;
        dinode.blocks = inode.blocks;
        let bytes = bincode::serialize(&dinode).unwrap();
        buf[..bytes.len()].copy_from_slice(&bytes);
        log_write(&bp);
    }

    // use typesystem to ensure lock in compile time!
    fn ilock(&mut self, inode: &mut MutexGuard<Inode>) {
        if inode.valid {
            return;
        }
        let block = get_buffer_block(block_of_inode(inode.inum), inode.dev.clone());
        let mut bp = block.write().unwrap();
        let offset = (inode.inum % IPB) as usize;
        let buf =
            &mut *(bp.as_mut::<[u8; BLOCK_SIZE as usize]>(offset * std::mem::size_of::<Dinode>()));
        let dinode: Dinode = bincode::deserialize(buf).unwrap();
        inode.ftype = match dinode.ftype {
            0 => FileType::None,
            1 => FileType::File,
            2 => FileType::Dir,
            _ => panic!("ilock: unknown file type"),
        };
        inode.nlink = dinode.nlink;
        inode.size = dinode.size;
        inode.blocks = dinode.blocks;
        inode.valid = true;
        if dinode.ftype == 0 {
            panic!("ilock: unallocated inode");
        }
    }

    // Drop a reference to an in-memory inode.
    // If that was the last reference, the inode table entry can be recycled.
    // If that was the last reference and the inode has no links to it, free the inode (and its content) on disk.
    // All calls to iput() must be inside a transaction in
    // case it has to free the inode.
    fn iput(&mut self, inode: Arc<Mutex<Inode>>) {
        let cache_guard = self.inodes.lock().unwrap();
        // last inode
        if Arc::strong_count(&inode) == 2 {
            let mut guard = inode.lock().unwrap();
            if guard.valid && guard.nlink == 0 {
                drop(cache_guard);
                guard.valid = false;
            }
        }
    }

    // Truncate inode (discard contents).
    // Caller must hold ip->lock.
    fn itrunc(&mut self, inode: &mut MutexGuard<Inode>) {
        for i in 0..NDIRECT {
            if inode.blocks[i as usize] != 0 {
                bfree(inode.dev.clone(), inode.blocks[i as usize]);
                inode.blocks[i as usize] = 0;
            }
        }

        if inode.blocks[NDIRECT as usize] != 0 {
            let block = get_buffer_block(inode.blocks[NDIRECT as usize], inode.dev.clone());
            let mut bp = block.write().unwrap();
            let buf = bp.as_mut::<[u32; NINDIRECT as usize]>(0);
            for i in 0..(NINDIRECT as usize) {
                if buf[i as usize] != 0 {
                    bfree(inode.dev.clone(), buf[i as usize]);
                }
            }
            bfree(inode.dev.clone(), inode.blocks[NDIRECT as usize]);
            inode.blocks[NDIRECT as usize] = 0;
        }

        if inode.blocks[(NDIRECT + 1) as usize] != 0 {
            let block = get_buffer_block(inode.blocks[NDIRECT as usize], inode.dev.clone());
            let mut bp = block.write().unwrap();
            let buf = bp.as_mut::<[u32; NINDIRECT as usize]>(0);
            for i in 0..NINDIRECT {
                if buf[i as usize] != 0 {
                    let b = buf[i as usize];
                    let block = get_buffer_block(buf[i as usize], inode.dev.clone());
                    let mut bp = block.write().unwrap();
                    let buf = bp.as_mut::<[u32; NINDIRECT as usize]>(0);
                    for j in 0..NINDIRECT {
                        if buf[j as usize] != 0 {
                            bfree(inode.dev.clone(), buf[j as usize]);
                        }
                    }
                    bfree(inode.dev.clone(), b);
                }
            }
            bfree(inode.dev.clone(), inode.blocks[(NDIRECT + 1) as usize]);
            inode.blocks[NDIRECT as usize] = 0;
        }
        inode.size = 0;
        self.iupdate(inode);
    }

    fn readi(
        &mut self,
        inode: &mut MutexGuard<Inode>,
        dst: &mut [u8],
        off: usize,
        len: usize,
    ) -> Result<(), String> {
        if off > inode.size as usize {
            return Err("readi: off/len out of range".to_string());
        }
        if off + len > MAXFILE as usize * BLOCK_SIZE as usize {
            return Err("readi: off/len out of range".to_string());
        }
        let mut start = off;
        while start < off + len {
            let addr = bmap(inode, start as u32 / BLOCK_SIZE).expect("readi: bmap");
            let block = get_buffer_block(addr, inode.dev.clone());
            let bp = block.read().unwrap();
            // copy
            let copy_bytes = std::cmp::min(
                BLOCK_SIZE as usize - (start % BLOCK_SIZE as usize),
                off + len - start,
            );
            let buf = bp.as_ref::<[u8; BLOCK_SIZE as usize]>(start % BLOCK_SIZE as usize);
            dst.copy_from_slice(&buf[..copy_bytes]);
            start += copy_bytes;
        }
        Ok(())
    }

    fn writei(
        &mut self,
        inode: &mut MutexGuard<Inode>,
        src: &[u8],
        off: usize,
        len: usize,
    ) -> Result<(), String> {
        if off > inode.size as usize {
            return Err("writei: off/len out of range".to_string());
        }
        let mut start = off;
        while start < off + len {
            let addr = bmap(inode, start as u32 / BLOCK_SIZE).expect("writei: bmap");
            let block = get_buffer_block(addr, inode.dev.clone());
            let mut bp = block.write().unwrap();
            // copy
            let copy_bytes = std::cmp::min(
                BLOCK_SIZE as usize - (start % BLOCK_SIZE as usize),
                off + len - start,
            );
            let buf = bp.as_mut::<[u8; BLOCK_SIZE as usize]>(start % BLOCK_SIZE as usize);
            buf[..copy_bytes].copy_from_slice(&src[start - off..start - off + copy_bytes]);
            start += copy_bytes;
        }
        if off + len > inode.size as usize {
            inode.size = (off + len) as u32;
            self.iupdate(inode);
        }
        Ok(())
    }

    fn namei(
        &mut self,
        dev: Arc<dyn BlockDevice>,
        path: PathBuf,
    ) -> Result<Arc<Mutex<Inode>>, String> {
        self.namex(dev.clone(), path, false)
    }

    fn dirlink(&mut self, dev: Arc<dyn BlockDevice>, dp: &mut MutexGuard<Inode>, name: String) {
        // check the name is not present
        if let Some((ip, _)) = self.dirlookup(dp, dev.clone(), name.clone()) {
            self.iput(ip);
        }

        // look for an empty direntry
        let mut de = None;
        for off in 0..dp.size as usize {
            let mut buf = [0u8; std::mem::size_of::<DirEntry>()];
            self.readi(dp, &mut buf, off, std::mem::size_of::<DirEntry>())
                .unwrap();
            // deserialize
            de = Some(bincode::deserialize::<DirEntry>(&buf).unwrap());
            if de.unwrap().inum == 0 {
                break;
            }
        }
    }

    fn nameiparent(
        &mut self,
        dev: Arc<dyn BlockDevice>,
        path: PathBuf,
    ) -> Result<Arc<Mutex<Inode>>, String> {
        self.namex(dev.clone(), path, true)
    }

    // helper for namei and nameiparent
    fn namex(
        &mut self,
        dev: Arc<dyn BlockDevice>,
        path: PathBuf,
        nameiparent: bool,
    ) -> Result<Arc<Mutex<Inode>>, String> {
        // check start with root
        let mut p = path.iter();
        if p.next().unwrap() != "/" {
            panic!("namex: path should start with /");
        }
        let mut prev = self.iget(dev.clone(), ROOTINO).unwrap();
        let mut next = Some(prev.clone());
        while let Some(name) = p.next() {
            prev = next.unwrap();
            let guard = prev.lock().unwrap();
            if guard.ftype != FileType::Dir {
                return Err(format!("namex: {} not a dir", name.to_str().unwrap()));
            }
            let res = self.dirlookup(
                &mut prev.lock().unwrap(),
                dev.clone(),
                name.to_str().unwrap().to_string(),
            );
            if res.is_none() && !nameiparent {
                return Err(format!("namex: {} not found", name.to_str().unwrap()));
            }
            let (inode, _) = res.unwrap();
            next = Some(inode.clone());
        }
        if nameiparent {
            Ok(prev)
        } else {
            Ok(next.unwrap())
        }
    }

    // look up the directory inode and return the inode
    pub fn dirlookup(
        &mut self,
        dp: &mut MutexGuard<Inode>,
        dev: Arc<dyn BlockDevice>,
        name: String,
    ) -> Option<(Arc<Mutex<Inode>>, u32)> {
        if dp.ftype != FileType::Dir {
            panic!("dirlookup: not a dir");
        }
        for off in (0..dp.size).step_by(std::mem::size_of::<DirEntry>()) {
            let mut buf = [0_u8; std::mem::size_of::<DirEntry>()];
            self.readi(dp, &mut buf, off as usize, std::mem::size_of::<DirEntry>())
                .unwrap();
            let entry: DirEntry = bincode::deserialize(&buf).unwrap();
            if entry.inum == 0 {
                continue;
            }
            if entry.name == name {
                return Some((self.iget(dev.clone(), entry.inum).unwrap(), off));
            }
        }
        None
    }

    pub fn create(
        &mut self,
        dev: Arc<dyn BlockDevice>,
        path: PathBuf,
        ftype: FileType,
    ) -> Result<Arc<Mutex<Inode>>, String> {
        let mut p = path.iter();
        if p.next().unwrap() != "/" {
            panic!("create: path should start with /");
        }

        // parent
        let parenti = self.nameiparent(dev.clone(), path.clone())?;
        let parent = parenti.lock().unwrap();
        let name = p.last().unwrap().to_str().unwrap().to_string();

        // already exists
        if let Some((i, _)) =
            self.dirlookup(&mut parenti.lock().unwrap(), dev.clone(), name.clone())
        {
            if i.lock().unwrap().ftype == ftype {
                return Ok(i);
            }
            return Err("create: file exists".to_string());
        }

        // alloc
        let inode = self.ialloc(dev.clone(), ftype).unwrap();
        let mut guard = inode.lock().unwrap();
        guard.ftype = ftype;
        guard.nlink = 1;
        self.iupdate(&guard);

        // if dir, create . and ..
        if ftype == FileType::Dir {}

        Err("Not Found".to_string())
    }
}

pub static mut ICACHE: Lazy<ICache> = Lazy::new(|| ICache::new());

pub fn ialloc(dev: Arc<dyn BlockDevice>, ftype: FileType) -> Option<Arc<Mutex<Inode>>> {
    unsafe { ICACHE.ialloc(dev, ftype) }
}

pub fn iput(inode: Arc<Mutex<Inode>>) {
    unsafe {
        ICACHE.iput(inode);
    }
}

pub fn iget(dev: Arc<dyn BlockDevice>, inum: u32) -> Option<Arc<Mutex<Inode>>> {
    unsafe { ICACHE.iget(dev, inum) }
}

// pub fn create(path: PathBuf, dev: Arc<dyn BlockDevice>, ftype: FileType) -> Result<(), String> {
//     info!("create: path: {:?}, ftype: {:?}", path, ftype as u32);
//     unsafe { ICACHE.create(dev, path, ftype) }
// }

// Disk Struct

// INode in disk, the size is 32!
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Dinode {
    pub dev: u32,                            // Device number, always 0
    pub ftype: u16,                          // File type
    pub nlink: u16,                          // Number of links to file
    pub size: u32,                           // Size of file (bytes)
    pub blocks: [u32; NDIRECT as usize + 2], // Pointers to blocks
}

// directory contains a sequence of entry
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DirEntry {
    pub inum: u32,
    pub name: String,
}

#[cfg(test)]
mod test {
    use std::{
        fs::{create_dir, File, OpenOptions},
        io::Write,
        thread,
    };

    use super::super::super::mkfs::mkfs;
    use super::*;
    use crate::fs::{
        filedisk::FileDisk,
        fs::BLOCK_SIZE,
        log::{begin_op, end_op, LOG_MANAGER},
    };
    #[test]
    fn test_dinode_size() {
        println!("size of Dinode: {}", std::mem::size_of::<Dinode>());
        assert_eq!(BLOCK_SIZE as usize % std::mem::size_of::<Dinode>(), 0);
    }

    #[test]
    fn test_ialloc() {
        mkfs("./test.img".into(), 1024 * 1024);
        // open test.img
        let file: File = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open("./test.img")
            .unwrap();
        // init
        let dev = Arc::new(FileDisk::new(file));
        unsafe {
            SB.init(dev.clone());
        }
        unsafe { LOG_MANAGER.init(&SB, dev.clone()) };

        let icache = ICache::new();
        // ialloc
        begin_op();
        assert_eq!(
            icache
                .ialloc(dev.clone(), FileType::File)
                .unwrap()
                .lock()
                .unwrap()
                .inum,
            2
        );
        assert_eq!(
            icache
                .ialloc(dev.clone(), FileType::File)
                .unwrap()
                .lock()
                .unwrap()
                .inum,
            3
        );
        end_op();
        thread::spawn(move || {
            begin_op();
            assert_eq!(
                icache
                    .ialloc(dev.clone(), FileType::File)
                    .unwrap()
                    .lock()
                    .unwrap()
                    .inum,
                4
            );
            end_op();
        })
        .join()
        .unwrap();
    }

    #[test]
    fn test_create() {
        mkfs("./test.img".into(), 1024 * 1024);
        // open test.img
        let file: File = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open("./test.img")
            .unwrap();
        // init
        let dev = Arc::new(FileDisk::new(file));
        unsafe {
            SB.init(dev.clone());
        }
        unsafe { LOG_MANAGER.init(&SB, dev.clone()) };
        // create
        begin_op();
        // assert_eq!(
        //     create(PathBuf::from("/test"), dev.clone(), FileType::File),
        //     Ok(())
        // );
        end_op();
    }
}
