use core::panic;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use log::info;
use once_cell::sync::Lazy;

use crate::fs::fs::BLOCK_SIZE;

use super::fs::{NINDIRECT, NINODES, ROOTINO};
use super::log::log_write;
use super::{
    buffer::get_buffer_block,
    fs::{BlockDevice, FileType, BPB, IPB, NAMESIZE, NDIRECT},
    superblock::SB,
};

// Disk Struct
#[repr(C)]
#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub struct DiskInode {
    pub dev: u32,                           // Device number, always 0
    pub ftype: u16,                         // File type
    pub nlink: u16,                         // Number of links to file
    pub size: u32,                          // Size of file (bytes)
    pub addrs: [u32; NDIRECT as usize + 1], // Pointers to blocks
}

// directory contains a sequence of entry
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
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
    true
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

// get the (block,offset) of inum
fn addr_of_inode(inum: u32) -> (u32, u32) {
    (
        inum / IPB + unsafe { SB.inodestart },
        inum % IPB * std::mem::size_of::<DiskInode>() as u32,
    )
}

// get the block containing the bitmap
fn block_of_bitmap(block: u32) -> u32 {
    block / BPB + unsafe { SB.bmapstart }
}

fn block_alloc(dev: Arc<dyn BlockDevice>) -> Option<u32> {
    for b in (0..unsafe { SB.size }).step_by(BPB as usize) {
        let bno = block_of_bitmap(b);
        let blk = get_buffer_block(bno, dev.clone());
        let mut guard = blk.write().unwrap();
        let mut buf = guard.read(0, |buf: &[u8; BLOCK_SIZE as usize]| *buf);
        for (i, byte) in buf.iter_mut().enumerate() {
            if *byte == 0xff {
                continue;
            }
            for j in 0..8 {
                let m = 1 << j;
                if *byte & m == 0 {
                    *byte |= m;
                    guard.write(i, |data: &mut u8| {
                        *data = *byte;
                    });
                    let buf = get_buffer_block(b + i as u32 * 8 + j, dev.clone());
                    let mut guard = buf.write().unwrap();
                    guard.write(0, |data: &mut [u8; BLOCK_SIZE as usize]| {
                        data.fill(0);
                    });
                    log_write(guard);
                    return Some(b + i as u32 * 8 + j);
                }
            }
        }
    }
    None
}

fn block_free(dev: Arc<dyn BlockDevice>, b: u32) {
    let bno = block_of_bitmap(b);
    let bi = b % BPB;
    get_buffer_block(bno, dev.clone())
        .write()
        .unwrap()
        .sync_write(bi as usize / 8, |data: &mut u8| {
            *data &= !(1 << (bi % 8));
        });
}

pub struct Inode {
    pub dev: Option<Arc<dyn BlockDevice>>,
    pub inum: u32,
    // the disk inode copy should be consistent with the disk
    // so we use a mutex to protect it
    // write/read the disk inode with modify_disk_inode/read_disk_inode
    // the dinode is not loaded(invalid in xv6), the dinode is None
    // the dinode will set to None while drop
    // if nlink == 0 and no other inode point to it(Arc::strong_count == 2(table and the drop routine))
    pub dinode: Mutex<Option<DiskInode>>, // inode copy
}

impl Inode {
    pub fn new() -> Self {
        Self {
            dev: None,
            inum: 0,
            dinode: Mutex::new(None),
        }
    }
}

impl Inode {
    fn read_disk_inode<V>(&self, f: impl FnOnce(&DiskInode) -> V) -> V {
        let (blk, off) = addr_of_inode(self.inum);
        get_buffer_block(blk, self.dev.as_ref().unwrap().clone())
            .read()
            .unwrap()
            .read(off as usize, f)
    }

    fn modify_disk_inode<V>(&self, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        let (blk, off) = addr_of_inode(self.inum);
        let binding = get_buffer_block(blk, self.dev.as_ref().unwrap().clone());
        let mut guard = binding.write().unwrap();
        let ret = guard.write(off as usize, f);
        log_write(guard);
        ret
    }

    pub fn truncate(dev: Arc<dyn BlockDevice>, dinode: &mut DiskInode) {
        // free the data blocks
        dinode
            .addrs
            .iter_mut()
            .take(NDIRECT as usize)
            .filter(|i| **i != 0)
            .for_each(|i| {
                block_free(dev.clone(), *i);
                *i = 0;
            });
        if dinode.addrs[NDIRECT as usize] > 0 {
            // read the indirect block
            let addrs = get_buffer_block(dinode.addrs[NDIRECT as usize], dev.clone())
                .read()
                .unwrap()
                .read(0, |addrs: &[u32; NINDIRECT as usize]| *addrs);
            addrs
                .iter()
                .take(NINDIRECT as usize)
                .filter(|i| **i != 0)
                .for_each(|i| block_free(dev.clone(), *i));
            block_free(dev.clone(), dinode.addrs[NDIRECT as usize]);
            dinode.addrs[NDIRECT as usize] = 0;
        }
    }
}

// design object:
// InodePtr is a pointer to Inode
// Every File should have a InodePtr
// Many file many have a ptr to same inode
#[derive(Clone)]
pub struct InodePtr(pub Arc<Inode>);

impl InodePtr {
    pub fn new() -> Self {
        Self(Arc::new(Inode::new()))
    }

    pub fn read_disk_inode<V>(&self, f: impl FnOnce(&DiskInode) -> V) -> V {
        // if the disk inode is not loaded, load it
        let mut guard = self.0.dinode.lock().unwrap();
        if guard.is_none() {
            let dinode = self.0.read_disk_inode(|dinode_ref| *dinode_ref);
            *guard = Some(dinode);
        }
        f(guard.as_ref().unwrap())
    }

    pub fn modify_disk_inode<V>(&self, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        let mut guard = self.0.dinode.lock().unwrap();
        if guard.is_none() {
            let dinode = self.0.read_disk_inode(|diskinode| *diskinode);
            *guard = Some(dinode);
        }
        let ret = f(guard.as_mut().unwrap());
        self.0.modify_disk_inode(|dinode| {
            *dinode = *guard.as_ref().unwrap();
        });
        ret
    }
}

pub struct InodePtrManager(Mutex<Vec<InodePtr>>);

impl InodePtrManager {
    pub fn new() -> Self {
        Self(Mutex::new((0..NINODES).map(|_| InodePtr::new()).collect()))
    }

    // mark an inode allocated in disk
    // and return an InodePtr with NonePtr
    pub fn inode_alloc(&self, dev: Arc<dyn BlockDevice>, ftype: FileType) -> Option<InodePtr> {
        for i in ROOTINO..unsafe { SB.ninodes } {
            let (bno, off) = addr_of_inode(i);
            let blk = get_buffer_block(bno, dev.clone());
            let mut blk_guard = blk.write().unwrap();
            let mut dinode = blk_guard.read(off as usize, |dinode: &DiskInode| *dinode);
            if dinode.ftype == FileType::Free as u16 {
                dinode.ftype = ftype as u16;
                blk_guard.write(off as usize, |diskinode: &mut DiskInode| {
                    *diskinode = dinode;
                });
                log_write(blk_guard);
                return Some(self.get_inode(dev.clone(), i));
            }
        }
        None
    }

    pub fn get_inode(&self, dev: Arc<dyn BlockDevice>, inum: u32) -> InodePtr {
        let mut guard = self.0.lock().unwrap();
        let mut empty = 0;
        for (i, inode) in guard.iter().enumerate() {
            if Arc::strong_count(&inode.0) > 1 && inode.0.inum == inum {
                return InodePtr(Arc::clone(&inode.0));
            }
            if empty == 0 && Arc::strong_count(&inode.0) == 1 {
                empty = i + 1;
            }
        }
        if empty == 0 {
            panic!("InodePtrManager::get_inode: no empty inode");
        }
        let i = empty - 1;
        guard[i] = InodePtr(Arc::new(Inode {
            dev: Some(dev.clone()),
            inum,
            dinode: Mutex::new(None),
        }));
        info!("InodePtrManager::get_inode: get inode {}", inum);
        return InodePtr(Arc::clone(&guard[i].0));
    }
}

impl Drop for InodePtr {
    fn drop(&mut self) {
        // the inode is in table and drop by caller
        // if the table drop it, will not truncate
        if Arc::strong_count(&self.0) == 2 {
            // lock the table
            info!("InodePtr::drop: drop inode {}", self.0.inum);
            let table_guard = unsafe { INODE_CACHE.0.lock().unwrap() };
            let mut dinode = self.0.dinode.lock().unwrap();
            if dinode.is_some() {
                let dinode = dinode.as_mut().unwrap();
                if dinode.nlink == 0 {
                    // truncate the inode
                    drop(table_guard);
                    Inode::truncate(self.0.dev.as_ref().unwrap().clone(), dinode);
                    info!("InodePtr::drop: truncate inode {}", self.0.inum);
                    // update on disk
                    self.0.modify_disk_inode(|dinode| {
                        dinode.ftype = FileType::Free as u16;
                        dinode.size = 0;
                    });
                }
            }
        }
    }
}

static mut INODE_CACHE: Lazy<InodePtrManager> = Lazy::new(|| InodePtrManager::new());

pub fn get_inode(dev: Arc<dyn BlockDevice>, inum: u32) -> InodePtr {
    unsafe { INODE_CACHE.get_inode(dev, inum) }
}

pub fn inode_alloc(dev: Arc<dyn BlockDevice>, ftype: FileType) -> Option<InodePtr> {
    unsafe { INODE_CACHE.inode_alloc(dev, ftype) }
}

pub fn find_child(
    dev: Arc<dyn BlockDevice>,
    diskinode: DiskInode,
    name: &str,
) -> Option<InodePtr> {
    let mut entries = Vec::new();
    for i in 0..NDIRECT {
        if diskinode.addrs[i as usize] != 0 {
            // read entries
            for j in (0 * std::mem::size_of::<DirEntry>()..BLOCK_SIZE as usize)
                .step_by(std::mem::size_of::<DirEntry>())
            {
                let entry = get_buffer_block(diskinode.addrs[i as usize], dev.clone())
                    .read()
                    .unwrap()
                    .read(j, |entry: &DirEntry| *entry);
                if entry.inum != 0 && !(i == 0 && j <= 2) {
                    entries.push(entry);
                }
            }
        }
    }
    // read indirect block
    if diskinode.addrs[NDIRECT as usize] != 0 {
        let addrs = get_buffer_block(diskinode.addrs[NDIRECT as usize], dev.clone())
            .read()
            .unwrap()
            .read(0, |addrs: &[u32; NINDIRECT as usize]| *addrs);
        for i in 0..NINDIRECT as usize {
            if addrs[i] != 0 {
                // read entries
                for j in (0..BLOCK_SIZE).step_by(std::mem::size_of::<DirEntry>()) {
                    let entry = get_buffer_block(addrs[i], dev.clone())
                        .read()
                        .unwrap()
                        .read(j as usize, |entry: &DirEntry| *entry);
                    if entry.inum != 0 {
                        entries.push(entry);
                    }
                }
            }
        }
    }
    for entry in entries {
        if namecmp(&entry.name, &name.to_string()) {
            return Some(get_inode(dev.clone(), entry.inum));
        }
    }
    None
}

pub fn find_inode(dev: Arc<dyn BlockDevice>, path: &PathBuf) -> Option<InodePtr> {
    let mut inode = get_inode(dev.clone(), ROOTINO);
    if *path == PathBuf::from("/") {
        return Some(inode);
    }
    if path.iter().next() != Some(&OsString::from("/")) {
        return None;
    }
    for name in path.iter().skip(1) {
        let dinode = inode.0.read_disk_inode(|diskinode| *diskinode);
        inode = match find_child(dev.clone(), dinode, name.to_str().unwrap()) {
            Some(inode) => inode,
            None => return None,
        }
    }
    Some(inode)
}

pub fn find_parent_inode(dev: Arc<dyn BlockDevice>, path: &PathBuf) -> Option<InodePtr> {
    let parent = PathBuf::from(path.parent().unwrap());
    find_inode(dev, &parent)
}

pub fn dirlink(dp: &mut InodePtr, name: &str, inum: u32) {
    // look for an empty dirent
    let mut de = DirEntry::default();
    let size = dp.0.read_disk_inode(|diskinode| diskinode.size as usize);
    let mut offset = 0;
    for off in (0..size).step_by(std::mem::size_of::<DirEntry>()) {
        let mut buf = [0u8; std::mem::size_of::<DirEntry>()];
        rinode(dp, &mut buf, off, std::mem::size_of::<DirEntry>());
        let entry =
            unsafe { std::mem::transmute::<[u8; std::mem::size_of::<DirEntry>()], DirEntry>(buf) };
        if entry.inum == 0 {
            de = entry;
            offset = off;
            break;
        }
        offset += std::mem::size_of::<DirEntry>();
    }

    de.inum = inum;
    nameassign(&mut de.name, &name.to_string());

    let src = unsafe { std::mem::transmute::<DirEntry, [u8; std::mem::size_of::<DirEntry>()]>(de) };
    winode(dp, &src, offset, src.len());
}

pub fn dirunlink(dp: &mut InodePtr, name: &str) -> Result<(), String> {
    let mut de = DirEntry::default();
    let size = dp.0.read_disk_inode(|diskinode| diskinode.size as usize);
    let mut offset = 0;
    for off in (0..size).step_by(std::mem::size_of::<DirEntry>()) {
        let mut buf = [0u8; std::mem::size_of::<DirEntry>()];
        rinode(dp, &mut buf, off, std::mem::size_of::<DirEntry>());
        let entry =
            unsafe { std::mem::transmute::<[u8; std::mem::size_of::<DirEntry>()], DirEntry>(buf) };
        if namecmp(&entry.name, &name.to_string()) {
            de = entry;
            offset = off;
            break;
        }
        offset += std::mem::size_of::<DirEntry>();
    }
    if de.inum == 0 {
        return Err("dirunlink: no entry".to_string());
    }
    de.inum = 0;
    nameassign(&mut de.name, &"".to_string());
    let src = unsafe { std::mem::transmute::<DirEntry, [u8; std::mem::size_of::<DirEntry>()]>(de) };
    winode(dp, &src, offset, src.len());
    Ok(())
}

pub fn create(dev: Arc<dyn BlockDevice>, path: &PathBuf, filetype: FileType) -> Option<InodePtr> {
    let parent_dir = find_parent_inode(dev.clone(), path);
    let mut dp = parent_dir.unwrap();
    let dp_dinode = dp.0.read_disk_inode(|diskinode| *diskinode);
    // alloc
    let dp_guard = dp.0.dinode.lock().unwrap();
    let ip = find_child(
        dev.clone(),
        dp_dinode,
        path.file_name().unwrap().to_str().unwrap(),
    );
    if let Some(inode) = ip {
        if inode.0.read_disk_inode(|diskinode| diskinode.ftype) == filetype as u16 {
            return Some(inode);
        }
    }
    if let Some(mut ip) = inode_alloc(dev.clone(), filetype) {
        // init
        ip.modify_disk_inode(|diskinode| {
            diskinode.nlink = 1;
            diskinode.size = 0;
        });
        // the inode ptr will not be dropped, so it's safe to lock stagely
        if filetype == FileType::Dir {
            // create . and ..
            let ip_inum = ip.0.inum;
            dirlink(&mut ip, ".", ip_inum);
            dirlink(&mut ip, "..", dp.0.inum);
        }
        let name = path.file_name().unwrap().to_str().unwrap();
        drop(dp_guard);
        dirlink(&mut dp, name, ip.0.inum);
        if filetype == FileType::Dir {
            // update parent dir size
            dp.modify_disk_inode(|diskinode| diskinode.nlink += 1);
        }
        Some(ip)
    } else {
        None
    }
}

// get the bn'th block of inode
pub fn block_map(diskinode: &mut DiskInode, dev: Arc<dyn BlockDevice>, mut offset_bn: u32) -> u32 {
    let mut addr;
    if offset_bn < NDIRECT {
        if diskinode.addrs[offset_bn as usize] == 0 {
            addr = block_alloc(dev.clone());
            diskinode.addrs[offset_bn as usize] = addr.unwrap();
        } else {
            addr = Some(diskinode.addrs[offset_bn as usize]);
        }
        return addr.unwrap();
    }
    offset_bn -= NDIRECT;
    if offset_bn < NINDIRECT {
        if diskinode.addrs[NDIRECT as usize] == 0 {
            addr = block_alloc(dev.clone());
            diskinode.addrs[NDIRECT as usize] = addr.unwrap();
        }
        let mut addrs = get_buffer_block(diskinode.addrs[NDIRECT as usize], dev.clone())
            .read()
            .unwrap()
            .read(0, |addrs: &[u32; NINDIRECT as usize]| *addrs);
        if addrs[offset_bn as usize] == 0 {
            addr = block_alloc(dev.clone());
            addrs[offset_bn as usize] = addr.unwrap();
            let blk = get_buffer_block(diskinode.addrs[NDIRECT as usize], dev.clone());
            let mut guard = blk.write().unwrap();
            guard.write(0, |data: &mut [u32; NINDIRECT as usize]| {
                    *data = addrs;
                });
            log_write(guard);
        } else {
            addr = Some(addrs[offset_bn as usize]);
        }
        return addr.unwrap();
    }
    0
}

pub fn rinode(ip: &mut InodePtr, dst: &mut [u8], mut off: usize, mut n: usize) -> usize {
    ip.modify_disk_inode(|diskinode| {
        let size = diskinode.size as usize;
        if off > size {
            return 0;
        }
        if off + n > size {
            n = size - off;
        }
        let mut tot = 0;
        while tot < n {
            let bp = get_buffer_block(
                block_map(
                    diskinode,
                    ip.0.dev.as_ref().unwrap().clone(),
                    off as u32 / BLOCK_SIZE,
                ),
                ip.0.dev.as_ref().unwrap().clone(),
            );
            let guard = bp.read().unwrap();
            let buf = guard.read(0, |buf: &[u8; BLOCK_SIZE as usize]| *buf);
            let m = std::cmp::min(n - tot, BLOCK_SIZE as usize - off % BLOCK_SIZE as usize);
            dst[tot..tot + m]
                .copy_from_slice(&buf[off % BLOCK_SIZE as usize..off % BLOCK_SIZE as usize + m]);
            tot += m;
            off += m;
        }
        tot
    })
}

pub fn winode(ip: &mut InodePtr, src: &[u8], mut off: usize, n: usize) -> usize {
    info!("winode: inum {} off {}, n {}", ip.0.inum, off, n);
    ip.modify_disk_inode(|diskinode| {
        let mut tot = 0;
        while tot < n {
            let bp = get_buffer_block(
                block_map(
                    diskinode,
                    ip.0.dev.as_ref().unwrap().clone(),
                    off as u32 / BLOCK_SIZE,
                ),
                ip.0.dev.as_ref().unwrap().clone(),
            );
            let mut guard = bp.write().unwrap();
            let mut buf = guard.read(0, |buf: &[u8; BLOCK_SIZE as usize]| *buf);
            let m = std::cmp::min(n - tot, BLOCK_SIZE as usize - off % BLOCK_SIZE as usize);
            buf[off % BLOCK_SIZE as usize..off % BLOCK_SIZE as usize + m]
                .copy_from_slice(&src[tot..tot + m]);
            guard.write(0, |data: &mut [u8; BLOCK_SIZE as usize]| {
                *data = buf;
            });
            log_write(guard);
            tot += m;
            off += m;
        }
        if off > diskinode.size as usize {
            diskinode.size = off as u32;
            info!(
                "winode: inode {} increase size {}",
                ip.0.inum, diskinode.size
            );
        }
        tot
    })
}

#[cfg(test)]
mod test {
    use std::{
        fs::{File, OpenOptions},
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    #[test]
    fn test_guard_and_ref() {
        let a = Some(Mutex::new(1));
        {
            let mut b = a.as_ref().unwrap().lock().unwrap();
            *b = 2;
        }
        let c = *a.as_ref().unwrap().lock().unwrap();
        assert!(c == 2)
    }

    use env_logger::{Builder, Target};

    use crate::fs::{
        buffer::{get_buffer_block, sync_all},
        filedisk::FileDisk,
        fs::{FileType, BLOCK_SIZE, ROOTINO},
        inode::DirEntry,
        log::{LOG_MANAGER, log_begin, log_end},
        superblock::SB,
    };

    use super::{create, winode, InodePtrManager};
    #[test]
    fn test_get_inode() {
        let file: File = OpenOptions::new()
            .read(true)
            .write(true)
            .create(false)
            .open("./test.img")
            .unwrap();
        let filedisk = Arc::new(FileDisk::new(file));
        let manager = InodePtrManager::new();
        let inode = manager.get_inode(filedisk.clone(), ROOTINO);
        // sb init
        unsafe { SB.init(filedisk.clone()) };
        // ls root
        let entries = inode.read_disk_inode(|diskinode| {
            let mut entries = Vec::new();
            for i in 0..super::NDIRECT {
                if diskinode.addrs[i as usize] != 0 {
                    // read entries
                    for j in (0..BLOCK_SIZE).step_by(std::mem::size_of::<DirEntry>()) {
                        let entry = get_buffer_block(diskinode.addrs[i as usize], filedisk.clone())
                            .read()
                            .unwrap()
                            .read(j as usize, |entry: &DirEntry| *entry);
                        if entry.inum != 0 {
                            entries.push(entry);
                        }
                    }
                }
            }
            entries
        });
        // . and ..
        assert!(entries.len() == 2);
        assert!(inode.0.inum == 1);
    }

    #[test]
    fn test_file_create() {
        // log
        Builder::new()
            .target(Target::Stdout)
            .is_test(true)
            .filter_level(log::LevelFilter::Info)
            .init();
        let file: File = OpenOptions::new()
            .read(true)
            .write(true)
            .create(false)
            .open("./test.img")
            .unwrap();
        let filedisk = Arc::new(FileDisk::new(file));
        unsafe { SB.init(filedisk.clone()) };
        unsafe { LOG_MANAGER.init(&SB, filedisk.clone()) };
        let manager = InodePtrManager::new();
        let inode = manager.inode_alloc(filedisk.clone(), FileType::File);
        log_begin();
        inode.unwrap().modify_disk_inode(|diskinode| {
            diskinode.nlink = 1;
            diskinode.size = 0;
        });
        log_end();
        sync_all();
    }

    #[test]
    fn test_bmap() {
        // log
        Builder::new()
            .target(Target::Stdout)
            .is_test(true)
            .filter_level(log::LevelFilter::Info)
            .init();
        let file: File = OpenOptions::new()
            .read(true)
            .write(true)
            .create(false)
            .open("./test.img")
            .unwrap();
        let filedisk = Arc::new(FileDisk::new(file));
        unsafe { SB.init(filedisk.clone()) };
        unsafe { LOG_MANAGER.init(&SB, filedisk.clone()) };
        let manager = InodePtrManager::new();
        let inode = manager.inode_alloc(filedisk.clone(), FileType::File);
        let addr = inode.unwrap().modify_disk_inode(|diskinode| {
            diskinode.nlink = 1;
            diskinode.size = 0;
            super::block_map(diskinode, filedisk, 0)
        });
        assert_eq!(addr, 197);
    }

    #[test]
    fn test_creat_read_write() {
        Builder::new()
            .target(Target::Stdout)
            .is_test(true)
            .filter_level(log::LevelFilter::Info)
            .init();
        let file: File = OpenOptions::new()
            .read(true)
            .write(true)
            .create(false)
            .open("./test.img")
            .unwrap();
        let filedisk = Arc::new(FileDisk::new(file));
        unsafe { SB.init(filedisk.clone()) };
        unsafe { LOG_MANAGER.init(&SB, filedisk.clone()) };
        // create
        let path = PathBuf::from("/test");
        log_begin();
        let mut testi = create(filedisk.clone(), &path, FileType::File).unwrap();
        winode(&mut testi, &[1, 2, 3, 4, 5, 6], 0, 6);
        let mut buf = [0; 6];
        super::rinode(&mut testi, &mut buf, 0, 6);
        assert_eq!(buf, [1, 2, 3, 4, 5, 6]);
        // test big file
        let mut buf = [1; 512 * 13];
        winode(&mut testi, &mut buf, 0, 512 * 13);
        let mut buf = [0; 512 * 13];
        super::rinode(&mut testi, &mut buf, 0, 512 * 13);
        assert_eq!(buf, [1; 512 * 13]);
        log_end();
        sync_all();
    }

    #[test]
    fn test_mkdir() {
        Builder::new()
            .target(Target::Stdout)
            .is_test(true)
            .filter_level(log::LevelFilter::Info)
            .init();
        let file: File = OpenOptions::new()
            .read(true)
            .write(true)
            .create(false)
            .open("./test.img")
            .unwrap();
        let filedisk = Arc::new(FileDisk::new(file));
        unsafe { SB.init(filedisk.clone()) };
        unsafe { LOG_MANAGER.init(&SB, filedisk.clone()) };
        // create
        let path = PathBuf::from("/test/");
        let _ = create(filedisk.clone(), &path, FileType::Dir).unwrap();
        let path = PathBuf::from("/test/test");
        let _ = create(filedisk.clone(), &path, FileType::File).unwrap();
        sync_all();
    }
}
