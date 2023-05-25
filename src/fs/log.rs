use std::sync::{Arc, Condvar, Mutex, RwLock, RwLockWriteGuard};

use log::info;
use once_cell::sync::Lazy;

use super::buffer::{get_buffer_block, BufferBlock};
use super::fs::*;
use super::superblock::SuperBlock;

// Contents of the log header block, used for both the on-disk header block
// and to keep track in memory of logged block before commit.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LogHeader {
    n: u32,                               // log len
    block: [u32; (LOGSIZE - 1) as usize], // block to write to
}

impl LogHeader {
    pub fn new() -> Self {
        Self {
            n: 0,
            block: [0; (LOGSIZE - 1) as usize],
        }
    }
}

// the log manager in memory
pub struct Log {
    dev: Option<Arc<dyn BlockDevice>>,
    head: u32, // head block
    size: u32, // log max size
    outstanding: u32,
    buffer_outstanding: Vec<Arc<RwLock<BufferBlock>>>, // for performance, the log buffer should in memory
    lh: LogHeader,                                     // log header
}

impl Log {
    pub fn new() -> Self {
        Self {
            dev: None,
            head: 0,
            size: 0,
            outstanding: 0,
            buffer_outstanding: Vec::new(),
            lh: LogHeader::new(),
        }
    }
    pub fn init(&mut self, sb: &SuperBlock, dev: Arc<dyn BlockDevice>) {
        self.dev = Some(dev.clone());
        self.head = sb.logstart;
        self.size = sb.nlog;
        self.recover();
    }

    pub fn read_head(&mut self) {
        let b = get_buffer_block(self.head, self.dev.as_ref().unwrap().clone());
        let buffer_guard = b.read().unwrap();
        let lh: &LogHeader = buffer_guard.as_ref(0);
        self.lh = *lh;
    }

    pub fn write_head(&mut self) {
        let b = get_buffer_block(self.head, self.dev.as_ref().unwrap().clone());
        let mut buffer_guard = b.write().unwrap();
        let lh: &mut LogHeader = buffer_guard.as_mut(0);
        *lh = self.lh;
        buffer_guard.sync();
    }

    pub fn write_log(&self) {
        for i in 0..self.lh.n {
            if self.lh.block[i as usize] == self.head + i + 1 {
                panic!(
                    "log area overlap, head: {}, i: {}, block_id : {}",
                    self.head, i, self.lh.block[i as usize]
                );
            }
            assert_ne!(self.lh.block[i as usize], self.head + i + 1);
            let to = get_buffer_block(self.head + i + 1, self.dev.as_ref().unwrap().clone());
            let mut to_guard = to.write().unwrap();
            let from = get_buffer_block(
                self.lh.block[i as usize],
                self.dev.as_ref().unwrap().clone(),
            );
            let from_guard = from.read().unwrap();
            let to_buf: &mut [u8; BLOCK_SIZE as usize] = to_guard.as_mut(0);
            to_buf.copy_from_slice(from_guard.as_ref::<[u8; BLOCK_SIZE as usize]>(0));
            to_guard.sync();
        }
    }

    pub fn install_commit(&mut self) {
        for i in 0..self.lh.n {
            if self.lh.block[i as usize] == self.head + i + 1 {
                panic!(
                    "log area overlap, head: {}, i: {}, block_id : {}",
                    self.head, i, self.lh.block[i as usize]
                );
            }
            assert_ne!(self.lh.block[i as usize], self.head + i + 1);
            let to = get_buffer_block(
                self.lh.block[i as usize],
                self.dev.as_ref().unwrap().clone(),
            );
            let mut to_guard = to.write().unwrap();
            let from = get_buffer_block(self.head + i + 1, self.dev.as_ref().unwrap().clone());
            let from_guard = from.read().unwrap();
            let to_buf: &mut [u8; BLOCK_SIZE as usize] = to_guard.as_mut(0);
            to_buf.copy_from_slice(from_guard.as_ref::<[u8; BLOCK_SIZE as usize]>(0));
            to_guard.sync();
        }
        self.buffer_outstanding.clear();
    }

    pub fn recover(&mut self) {
        self.read_head();
        self.install_commit();
        self.lh.n = 0;
        self.write_head();
    }

    pub fn commit(&mut self) {
        if self.lh.n > 0 {
            // write commit record to disk
            self.write_log(); // write cached block to log block
            self.write_head(); // write log header to disk
            self.install_commit(); // write log block to dst block
            self.lh.n = 0; // ? why jetbrains mono is not mono (in vsc)?
                           // fuck jetbrains
            self.write_head(); // the true block is written, write empty head to disk
        }
    }
}

pub struct LogManager(Mutex<Log>, Condvar);

impl LogManager {
    pub fn new() -> Self {
        LogManager(Mutex::new(Log::new()), Condvar::new())
    }

    pub fn init(&mut self, sb: &SuperBlock, dev: Arc<dyn BlockDevice>) {
        self.0.lock().unwrap().init(sb, dev);
    }

    pub fn begin_op(&self) {
        let mut log_guard = self.0.lock().unwrap();
        loop {
            if (log_guard.lh.n + (log_guard.outstanding + 1) * MAXOPBLOCKS) > log_guard.size {
                // this op might exhaust log space; wait for commit.
                log_guard = self.1.wait(log_guard).unwrap();
            } else {
                log_guard.outstanding += 1;
                break;
            }
        }
    }

    pub fn end_op(&self) {
        // let mut do_commit = false;
        let mut log_guard = self.0.lock().unwrap();
        assert!(log_guard.outstanding > 0);
        log_guard.outstanding -= 1;
        if log_guard.outstanding == 0 {
            log_guard.commit();
        }
        self.1.notify_all();
    }

    pub fn write(&mut self, buffer: &RwLockWriteGuard<BufferBlock>) {
        let mut log_guard = self.0.lock().unwrap();
        assert!(log_guard.lh.n < LOGSIZE as u32);
        assert!(log_guard.outstanding > 0);
        let mut n = log_guard.lh.n;

        // log absorption
        for i in 0..log_guard.lh.n {
            if log_guard.lh.block[i as usize] == buffer.id() {
                n = i;
                break;
            }
        }

        log_guard.lh.block[n as usize] = buffer.id();

        if n == log_guard.lh.n {
            log_guard.lh.n += 1;
            // get the buffer block
            let dev = log_guard.dev.as_ref().unwrap().clone();
            log_guard
                .buffer_outstanding
                .push(get_buffer_block(buffer.id(), dev));
        }
    }
}

pub static mut LOG_MANAGER: Lazy<LogManager> = Lazy::new(|| LogManager::new());

pub fn log_write(buffer: &RwLockWriteGuard<BufferBlock>) {
    info!("log_write: {}", buffer.id());
    unsafe {
        LOG_MANAGER.write(buffer);
    }
}

pub fn begin_op() {
    unsafe {
        LOG_MANAGER.begin_op();
    }
}

pub fn end_op() {
    unsafe {
        LOG_MANAGER.end_op();
    }
}

#[cfg(test)]
mod test {
    use std::{
        fs::{File, OpenOptions},
        io::Write,
        os::unix::prelude::FileExt,
        sync::Arc,
        thread,
    };

    use super::super::filedisk::FileDisk;
    use super::*;
    use crate::fs::fs::BlockDevice;

    #[test]
    fn test_delay_write() {
        let mut file: File = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open("./test.img")
            .unwrap();
        file.set_len(1024 * 1024).unwrap();
        file.write_all(&[0 as u8; 1024 * 1024]).unwrap();
        let filedisk = Arc::new(FileDisk::new(file));
        filedisk.write_block(0, &[0 as u8; 512]);
        filedisk.write_block(1, &[1 as u8; 512]);
        let src = get_buffer_block(0, filedisk.clone());
        let dst = get_buffer_block(1, filedisk.clone());
        let src_guard = src.read().unwrap();
        let mut dst_guard = dst.write().unwrap();
        let src_buffer: &[u8; 512] = src_guard.as_ref(0);
        let dst_buffer: &mut [u8; 512] = dst_guard.as_mut(0);
        dst_buffer.copy_from_slice(src_buffer);
        assert_eq!(dst_buffer, &[0 as u8; 512]);
        // read from file
        let file: File = OpenOptions::new()
            .read(true)
            .write(false)
            .open("./test.img")
            .unwrap();
        let mut buffer = [0 as u8; 512];
        file.read_exact_at(&mut buffer, 512).unwrap();
        // the write is delayed, so it's still 1
        assert_eq!(buffer, [1 as u8; 512]);
        // test dirty bit
        dst_guard.sync();
        file.read_exact_at(&mut buffer, 512).unwrap();
        // the write is flush, so it's 0!
        assert_eq!(buffer, [0 as u8; 512]);
    }

    #[test]
    fn test_read_write_head() {
        let mut file: File = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open("./test.img")
            .unwrap();
        file.set_len(1024 * 1024).unwrap();
        file.write_all(&[0 as u8; 1024 * 1024]).unwrap();
        let filedisk = Arc::new(FileDisk::new(file));
        let mut sb = SuperBlock::new();
        sb.logstart = 2;
        sb.nlog = LOGSIZE;
        let mut log = Log::new();
        log.init(&sb, filedisk.clone());
        let mut lh = LogHeader::new();
        lh.n = 1;
        log.lh = lh;
        log.write_head();
        drop(log);
        let mut log = Log::new();
        log.init(&sb, filedisk.clone());
        // recover will empty the log
        assert_eq!(log.lh.n, 0);
    }

    #[test]
    fn test_mp() {
        let mut file: File = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open("./test.img")
            .unwrap();
        file.set_len(1024 * 1024).unwrap();
        file.write_all(&[0 as u8; 1024 * 1024]).unwrap();
        let filedisk = Arc::new(FileDisk::new(file));
        let mut sb = SuperBlock::new();
        sb.logstart = 2;
        sb.nlog = LOGSIZE;
        let mut log = Log::new();
        log.init(&sb, filedisk.clone());
        let mut lh = LogHeader::new();
        lh.n = 1;
        log.lh = lh;
        log.write_head();
        unsafe { LOG_MANAGER.init(&sb, filedisk.clone()) };
        let mut handles = Vec::new();
        for i in 0..100u8 {
            let filedisk = filedisk.clone();
            let handle = thread::spawn(move || {
                unsafe { LOG_MANAGER.begin_op() };
                {
                    let buffer = get_buffer_block(i as u32 + 3 + LOGSIZE, filedisk.clone());
                    let mut buffer_guard = buffer.write().unwrap();
                    let b: &mut [u8; 512] = buffer_guard.as_mut(0);
                    b[0] = i;
                    unsafe { LOG_MANAGER.write(&buffer_guard) };
                }
                unsafe { LOG_MANAGER.end_op() };
            });
            handles.push(handle);
        }

        handles
            .into_iter()
            .for_each(|handle| handle.join().unwrap());

        for i in 0..100u8 {
            let buffer = get_buffer_block(i as u32 + 3 + LOGSIZE, filedisk.clone());
            let buffer_guard = buffer.read().unwrap();
            let buffer: &[u8; 512] = buffer_guard.as_ref(0);
            assert_eq!(buffer[0], i);
        }
    }
}
