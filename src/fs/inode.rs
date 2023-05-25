use log::info;
use once_cell::sync::Lazy;
use std::collections::linked_list;
use std::sync::RwLockWriteGuard;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};

use super::buffer::BufferBlock;
use super::file::FileType;
use super::fs::{MAXFILE, NAMESIZE};
use super::{
    buffer::get_buffer_block,
    fs::{BlockDevice, BLOCK_SIZE, BPB, IPB, NDIRECT, NINDIRECT, NINODES, ROOTINO},
    log::log_write,
    superblock::SB,
};

pub struct InodeData {
    valid: bool,                         // Valid?
    ftype: FileType,                     // FileType
    nlink: u16,                          // Number of links to file
    size: u32,                           // Size of file (bytes)
    blocks: [u32; NDIRECT as usize + 2], // Pointers to blocks
}

// INode in memory
pub struct Inode {
    dev: Arc<dyn BlockDevice>,   // Device
    inum: u32,                   // Inode number
    data: Arc<Mutex<InodeData>>, // Inode content
}

pub struct ICache {
    inodes: Arc<Mutex<Vec<Option<Arc<Inode>>>>>,
}

fn block_of_inode(inum: u32) -> u32 {
    (inum / IPB) + unsafe { SB.inodestart }
}

fn block_of_bitmap(b: u32) -> u32 {
    (b / BPB) + unsafe { SB.bmapstart }
}

fn bzero(bp: &mut RwLockWriteGuard<BufferBlock>) {
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
            bzero(&mut bp);
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
pub fn bmap(
    inode: &mut MutexGuard<InodeData>,
    bn: u32,
    dev: Arc<dyn BlockDevice>,
) -> Result<u32, String> {
    let mut bn = bn;
    if bn < NDIRECT {
        if inode.blocks[bn as usize] == 0 {
            inode.blocks[bn as usize] = balloc(dev.clone());
        }
        return Ok(inode.blocks[bn as usize]);
    }
    bn -= NDIRECT;
    if bn < NINDIRECT {
        if inode.blocks[NDIRECT as usize] == 0 {
            inode.blocks[NDIRECT as usize] = balloc(dev.clone());
        }
        let block = get_buffer_block(inode.blocks[NDIRECT as usize], dev.clone());
        let mut bp = block.write().unwrap();
        let buf = bp.as_mut::<[u32; NINDIRECT as usize]>(0);
        if buf[bn as usize] == 0 {
            buf[bn as usize] = balloc(dev.clone());
        }
        return Ok(buf[bn as usize]);
    }
    bn -= NINDIRECT;
    if bn < NINDIRECT * NINDIRECT {
        if inode.blocks[(NDIRECT + 1) as usize] == 0 {
            inode.blocks[(NDIRECT + 1) as usize] = balloc(dev.clone());
        }
        let block = get_buffer_block(inode.blocks[(NDIRECT + 1) as usize], dev.clone());
        let mut bp: std::sync::RwLockWriteGuard<crate::fs::buffer::BufferBlock> =
            block.write().unwrap();
        let buf = bp.as_mut::<[u32; NINDIRECT as usize]>(0);
        let block = bn / NINDIRECT;
        if buf[block as usize] == 0 {
            buf[block as usize] = balloc(dev.clone());
        }
        let block = get_buffer_block(buf[block as usize], dev.clone());
        let mut bp = block.write().unwrap();
        let buf = bp.as_mut::<[u32; NINDIRECT as usize]>(0);
        if buf[bn as usize] == 0 {
            buf[bn as usize] = balloc(dev.clone());
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
    fn ialloc(&self, dev: Arc<dyn BlockDevice>, ftype: FileType) -> Option<Arc<Inode>> {
        for i in 1..NINODES {
            let block = get_buffer_block(block_of_inode(i), dev.clone());
            let mut bp = block.write().unwrap();
            let dinode: &mut Dinode = bp.as_mut(i as usize * std::mem::size_of::<Dinode>());
            if dinode.ftype == 0 {
                info!("ialloc: alloc inode {}", i);
                dinode.ftype = ftype as u16;
                log_write(&bp);
                return self.iget(dev, i);
            }
        }
        panic!("ialloc: no inodes");
    }

    // Find the inode with number inum on device dev
    // and return the in-memory copy. Does not lock
    // the inode and does not read it from disk.
    fn iget(&self, dev: Arc<dyn BlockDevice>, inum: u32) -> Option<Arc<Inode>> {
        let mut guard = self.inodes.lock().unwrap();

        let mut empty = 0;
        for i in 1..NINODES {
            // if in icache
            if let Some(inode) = &guard[i as usize] {
                if inode.inum == inum {
                    return Some(inode.clone());
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

        let inode = Arc::new(Inode {
            dev: dev.clone(),
            inum: inum,
            data: Arc::new(Mutex::new(InodeData {
                ftype: FileType::None,
                valid: false,
                nlink: 0,
                size: 0,
                blocks: [0; NDIRECT as usize + 2],
            })),
        });
        guard[empty as usize] = Some(inode.clone());
        Some(inode)
    }

    fn iupdate(&mut self, inode: &MutexGuard<InodeData>, inum: u32, dev: Arc<dyn BlockDevice>) {
        info!("iupdate: inum = {}", inum);
        let block = get_buffer_block(block_of_inode(inum), dev.clone());
        let mut bp = block.write().unwrap();
        let offset = (inum % IPB) as usize;
        let dinode: &mut Dinode = bp.as_mut(offset * std::mem::size_of::<Dinode>());
        dinode.ftype = inode.ftype as u16;
        dinode.nlink = inode.nlink;
        dinode.size = inode.size;
        dinode.blocks = inode.blocks;
        log_write(&bp);
    }

    // use typesystem to ensure lock in compile time!
    fn ilock(&mut self, inode: &mut MutexGuard<InodeData>, inum: u32, dev: Arc<dyn BlockDevice>) {
        if inode.valid {
            return;
        }
        let block = get_buffer_block(block_of_inode(inum), dev.clone());
        let bp = block.write().unwrap();
        let offset = (inum % IPB) as usize;
        let dinode: &Dinode = bp.as_ref(offset * std::mem::size_of::<Dinode>());
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
    fn iput(&mut self, inode: Arc<Inode>) {
        let cache_guard = self.inodes.lock().unwrap();
        // last inode
        // table + callee
        if Arc::strong_count(&inode) == 2 {
            let mut guard = inode.data.lock().unwrap();
            if guard.valid && guard.nlink == 0 {
                drop(cache_guard);
                self.itrunc(&mut guard, inode.inum, inode.dev.clone());
                guard.valid = false;
            }
        }
    }

    // Truncate inode (discard contents).
    // Caller must hold ip->lock.
    fn itrunc(&mut self, inode: &mut MutexGuard<InodeData>, inum: u32, dev: Arc<dyn BlockDevice>) {
        for i in 0..NDIRECT {
            if inode.blocks[i as usize] != 0 {
                bfree(dev.clone(), inode.blocks[i as usize]);
                inode.blocks[i as usize] = 0;
            }
        }

        if inode.blocks[NDIRECT as usize] != 0 {
            let block = get_buffer_block(inode.blocks[NDIRECT as usize], dev.clone());
            let mut bp = block.write().unwrap();
            let buf = bp.as_mut::<[u32; NINDIRECT as usize]>(0);
            for i in 0..(NINDIRECT as usize) {
                if buf[i as usize] != 0 {
                    bfree(dev.clone(), buf[i as usize]);
                }
            }
            bfree(dev.clone(), inode.blocks[NDIRECT as usize]);
            inode.blocks[NDIRECT as usize] = 0;
        }

        if inode.blocks[(NDIRECT + 1) as usize] != 0 {
            let block = get_buffer_block(inode.blocks[NDIRECT as usize], dev.clone());
            let mut bp = block.write().unwrap();
            let buf = bp.as_mut::<[u32; NINDIRECT as usize]>(0);
            for i in 0..NINDIRECT {
                if buf[i as usize] != 0 {
                    let b = buf[i as usize];
                    let block = get_buffer_block(buf[i as usize], dev.clone());
                    let mut bp = block.write().unwrap();
                    let buf = bp.as_mut::<[u32; NINDIRECT as usize]>(0);
                    for j in 0..NINDIRECT {
                        if buf[j as usize] != 0 {
                            bfree(dev.clone(), buf[j as usize]);
                        }
                    }
                    bfree(dev.clone(), b);
                }
            }
            bfree(dev.clone(), inode.blocks[(NDIRECT + 1) as usize]);
            inode.blocks[NDIRECT as usize] = 0;
        }
        inode.size = 0;
        self.iupdate(&inode, inum, dev);
    }

    fn readi(
        &mut self,
        dev: Arc<dyn BlockDevice>,
        inode: &mut MutexGuard<InodeData>,
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
            let addr = bmap(inode, start as u32 / BLOCK_SIZE, dev.clone()).expect("readi: bmap");
            let block = get_buffer_block(addr, dev.clone());
            let bp = block.read().unwrap();
            // copy
            let copy_bytes = std::cmp::min(
                BLOCK_SIZE as usize - (start % BLOCK_SIZE as usize),
                off + len - start,
            );
            let buf = bp.as_ref::<[u8; BLOCK_SIZE as usize]>(0);
            // dst.copy_from_slice(&buf[..copy_bytes]);
            dst[0..copy_bytes].copy_from_slice(
                &buf[start % BLOCK_SIZE as usize..start % BLOCK_SIZE as usize + copy_bytes],
            );
            start += copy_bytes;
        }
        Ok(())
    }

    fn writei(
        &mut self,
        dev: Arc<dyn BlockDevice>,
        inode: &mut MutexGuard<InodeData>,
        inum: u32,
        src: &[u8],
        off: usize,
        len: usize,
    ) -> Result<usize, String> {
        info!("writei: inum: {}, off: {}, len: {}", inum, off, len);
        info!("writei: src: {:?}", src);
        if off > inode.size as usize {
            return Err("writei: off/len out of range".to_string());
        }
        let mut start = off;
        while start < off + len {
            let addr = bmap(inode, start as u32 / BLOCK_SIZE, dev.clone()).expect("writei: bmap");
            let block = get_buffer_block(addr, dev.clone());
            let mut bp = block.write().unwrap();
            // copy
            let copy_bytes = std::cmp::min(
                BLOCK_SIZE as usize - (start % BLOCK_SIZE as usize),
                off + len - start,
            );
            let buf = bp.as_mut::<[u8; BLOCK_SIZE as usize]>(0);
            buf[start..start + copy_bytes]
                .copy_from_slice(&src[start - off..start - off + copy_bytes]);
            start += copy_bytes;
            log_write(&bp);
        }
        if off + len > inode.size as usize {
            inode.size = (off + len) as u32;
        }
        self.iupdate(inode, inum, dev.clone());
        Ok(len)
    }

    fn namei(&mut self, dev: Arc<dyn BlockDevice>, path: PathBuf) -> Option<Arc<Inode>> {
        self.namex(dev.clone(), path, false)
    }

    // Write a new directory entry (name, inum) into the directory dp.
    // Returns 0 on success, -1 on failure (e.g. out of disk blocks).
    fn dirlink(
        &mut self,
        dev: Arc<dyn BlockDevice>,
        dp: &mut MutexGuard<InodeData>,
        name: String,
        selfi: u32,
        linki: u32,
    ) -> Result<(), String> {
        info!("dirlink: name: {}, inum: {}", name, linki);
        // check the name is not present
        if let Some((ip, _)) = self.dirlookup(dp, dev.clone(), name.clone()) {
            self.iput(ip);
        }

        let mut off: usize = 0;
        // look for an empty direntry off set
        while off < dp.size as usize {
            let mut buf = [0u8; std::mem::size_of::<DirEntry>()];
            self.readi(
                dev.clone(),
                dp,
                &mut buf,
                off,
                std::mem::size_of::<DirEntry>(),
            )?;
            let de = unsafe {
                std::mem::transmute::<[u8; std::mem::size_of::<DirEntry>()], DirEntry>(buf)
            };
            if de.inum == 0 {
                break;
            }
            off += std::mem::size_of::<DirEntry>();
        }
        let mut de = DirEntry::default();
        de.inum = linki;
        nameassign(&mut de.name, &name);
        // get the byte
        let src =
            unsafe { std::mem::transmute::<DirEntry, [u8; std::mem::size_of::<DirEntry>()]>(de) };
        self.writei(
            dev.clone(),
            dp,
            selfi,
            src.as_ref(),
            off,
            std::mem::size_of::<DirEntry>(),
        )?;
        Ok(())
    }

    fn nameiparent(&mut self, dev: Arc<dyn BlockDevice>, path: PathBuf) -> Option<Arc<Inode>> {
        self.namex(dev.clone(), path, true)
    }

    // helper for namei and nameiparent
    fn namex(
        &mut self,
        dev: Arc<dyn BlockDevice>,
        path: PathBuf,
        nameiparent: bool,
    ) -> Option<Arc<Inode>> {
        // check start with root
        let mut p = path.iter();
        if p.next().unwrap() != "/" {
            panic!("namex: path should start with /");
        }

        let last = path.file_name().unwrap();
        let mut ip = self.iget(dev.clone(), ROOTINO).unwrap();
        while let Some(name) = p.next() {
            let mut guard = ip.data.lock().unwrap();
            self.ilock(&mut guard, ip.inum, dev.clone());
            if guard.ftype != FileType::Dir {
                return None;
            }
            if nameiparent && name == last {
                drop(guard);
                return Some(ip.clone());
            }
            let next = self.dirlookup(&mut guard, dev.clone(), name.to_str().unwrap().to_string());
            if next.is_none() {
                drop(guard);
                self.iput(ip);
                return None;
            }
            drop(guard);
            self.iput(ip);
            ip = next.unwrap().0;
        }
        if nameiparent {
            self.iput(ip.clone());
            None
        } else {
            Some(ip)
        }
    }

    // look up the directory inode and return the inode
    fn dirlookup(
        &mut self,
        dp: &mut MutexGuard<InodeData>,
        dev: Arc<dyn BlockDevice>,
        name: String,
    ) -> Option<(Arc<Inode>, u32)> {
        if dp.ftype != FileType::Dir {
            panic!("dirlookup: not a dir");
        }
        for off in (0..dp.size).step_by(std::mem::size_of::<DirEntry>()) {
            let mut buf = [0_u8; std::mem::size_of::<DirEntry>()];
            self.readi(
                dev.clone(),
                dp,
                &mut buf,
                off as usize,
                std::mem::size_of::<DirEntry>(),
            )
            .unwrap();
            let entry = unsafe {
                std::mem::transmute::<[u8; std::mem::size_of::<DirEntry>()], DirEntry>(buf)
            };
            if entry.inum == 0 {
                continue;
            }
            // cmp name
            if namecmp(&entry.name, &name) {
                return Some((self.iget(dev.clone(), entry.inum).unwrap(), off as u32));
            }
        }
        None
    }

    pub fn create(
        &mut self,
        dev: Arc<dyn BlockDevice>,
        path: PathBuf,
        ftype: FileType,
    ) -> Result<Arc<Inode>, String> {
        let mut p = path.iter();
        if p.next().unwrap() != "/" {
            panic!("create: path should start with /");
        }

        // parent
        let parenti = self.nameiparent(dev.clone(), path.clone()).unwrap();
        let mut dp = parenti.data.lock().unwrap();
        let name = p.last().unwrap().to_str().unwrap().to_string();

        // already exists
        if let Some((i, _)) = self.dirlookup(&mut dp, dev.clone(), name.clone()) {
            if i.data.lock().unwrap().ftype == ftype {
                return Ok(i);
            }
            return Err("create: file exists".to_string());
        }

        // alloc
        let inode = self.ialloc(dev.clone(), ftype).unwrap();
        let mut ip = inode.data.lock().unwrap();
        ip.ftype = ftype;
        ip.nlink = 1;
        self.iupdate(&ip, inode.inum, inode.dev.clone());

        // if dir, create . and ..
        if ftype == FileType::Dir {
            let selfi = inode.inum;
            let linki = inode.inum;
            self.dirlink(dev.clone(), &mut ip, ".".to_string(), selfi, linki)?;
            let linki = parenti.inum;
            self.dirlink(dev.clone(), &mut ip, "..".to_string(), selfi, linki)?;
        }

        let selfi = parenti.inum;
        let linki = inode.inum;
        self.dirlink(dev.clone(), &mut dp, name, selfi, linki)?;

        if ftype == FileType::Dir {
            // for ..
            dp.nlink += 1;
            self.iupdate(&dp, parenti.inum, parenti.dev.clone());
        }

        drop(dp);
        iput(parenti);
        Ok(inode.clone())
    }
}

pub static mut ICACHE: Lazy<ICache> = Lazy::new(|| ICache::new());

pub fn ialloc(dev: Arc<dyn BlockDevice>, ftype: FileType) -> Option<Arc<Inode>> {
    unsafe { ICACHE.ialloc(dev, ftype) }
}

pub fn iput(inode: Arc<Inode>) {
    info!("iput: inum: {:?}", inode.inum);
    unsafe {
        ICACHE.iput(inode);
    }
}

pub fn iget(dev: Arc<dyn BlockDevice>, inum: u32) -> Option<Arc<Inode>> {
    info!("iget: inum: {:?}", inum);
    unsafe { ICACHE.iget(dev, inum) }
}

// Disk Struct

#[repr(C)]
#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub struct Dinode {
    pub dev: u32,                            // Device number, always 0
    pub ftype: u16,                          // File type
    pub nlink: u16,                          // Number of links to file
    pub size: u32,                           // Size of file (bytes)
    pub blocks: [u32; NDIRECT as usize + 2], // Pointers to blocks
}

// directory contains a sequence of entry
#[repr(C)]
#[derive(Debug, Default)]
pub struct DirEntry {
    pub inum: u32,
    pub name: [u8; NAMESIZE as usize],
}

pub fn namecmp(s: &[u8], t: &String) -> bool {
    let mut i = 0;
    for c in t.chars() {
        if i >= s.len() {
            return false;
        }
        if s[i] != c as u8 {
            return false;
        }
        i += 1;
    }
    return i == s.len();
}

pub fn nameassign(s: &mut [u8], t: &String) {
    let mut i = 0;
    for c in t.chars() {
        if i >= s.len() {
            panic!("nameassign: name too long");
        }
        s[i] = c as u8;
        i += 1;
    }
    while i < s.len() {
        s[i] = 0;
        i += 1;
    }
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
    fn test_name_operations() {
        let mut buf = [0_u8; 16];
        buf[0] = 'h' as u8;
        buf[1] = 'e' as u8;
        buf[2] = 'l' as u8;
        buf[3] = 'l' as u8;
        buf[4] = 'o' as u8;
        let name = "hello".to_string();
        assert_eq!(namecmp(&buf, &name), true);
    }

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
        assert_eq!(icache.ialloc(dev.clone(), FileType::File).unwrap().inum, 2);
        assert_eq!(icache.ialloc(dev.clone(), FileType::File).unwrap().inum, 3);
        end_op();
        thread::spawn(move || {
            begin_op();
            assert_eq!(icache.ialloc(dev.clone(), FileType::File).unwrap().inum, 4);
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
        let path = PathBuf::from("/test");
        let ftype = FileType::File;
        unsafe { ICACHE.create(dev.clone(), path, ftype) }.unwrap();
        end_op();
    }
}
