use std::sync::{Arc, Condvar, Mutex, RwLock, RwLockWriteGuard};

use log::{info, debug};
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
    lock: Mutex<()>,
    cond: Condvar,
    dev: Option<Arc<dyn BlockDevice>>,
    head: u32, // head block
    size: u32, // log max size
    outstanding: u32,
    committing: bool,
    buffer_outstanding: Vec<Arc<RwLock<BufferBlock>>>, // for performance, the log buffer should in memory
    lh: LogHeader,                                     // log header
}

pub static mut LOG : Lazy<Log> = Lazy::new(|| Log::new());

impl Log {
    fn new() -> Self {
        Self {
            dev: None,
            lock: Mutex::new(()),
            cond: Condvar::new(),
            head: 0,
            size: 0,
            outstanding: 0,
            committing: false,
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

    fn read_head(&mut self) {
        let b = get_buffer_block(self.head, self.dev.as_ref().unwrap().clone());
        b.read().unwrap().read(0, |lh: &LogHeader| {
            self.lh = *lh;
        });
    }

    fn write_head(&mut self) {
        info!("{:?} write head", std::thread::current().id());
        let b = get_buffer_block(self.head, self.dev.as_ref().unwrap().clone());
        b.write().unwrap().sync_write(0, |lh: &mut LogHeader| {
            *lh = self.lh;
        });
    }

    fn write_log(&self) {
        (0..self.lh.n).for_each(|i| {
            assert_ne!(self.lh.block[i as usize], self.head + i + 1);
            get_buffer_block(self.head + i + 1, self.dev.as_ref().unwrap().clone())
                .write()
                .unwrap()
                .sync_write(0, |buf: &mut [u8; BLOCK_SIZE as usize]| {
                    buf.copy_from_slice(
                        &get_buffer_block(
                            self.lh.block[i as usize],
                            self.dev.as_ref().unwrap().clone(),
                        )
                        .read()
                        .unwrap()
                        .read(0, |f: &[u8; BLOCK_SIZE as usize]| f.clone()),
                    )
                });
        })
    }

    fn install_commit(&mut self) {
        (0..self.lh.n).for_each(|i| {
            assert_ne!(self.lh.block[i as usize], self.head + i + 1);
            get_buffer_block(
                self.lh.block[i as usize],
                self.dev.as_ref().unwrap().clone(),
            )
            .write()
            .unwrap()
            .sync_write(0, |buf: &mut [u8; BLOCK_SIZE as usize]| {
                buf.copy_from_slice(
                    &get_buffer_block(self.head + i + 1, self.dev.as_ref().unwrap().clone())
                        .read()
                        .unwrap()
                        .read(0, |f: &[u8; BLOCK_SIZE as usize]| f.clone()),
                )
            });
        });
        self.buffer_outstanding.clear();
    }

    fn recover(&mut self) {
        info!("{:?} recover", std::thread::current().id());
        self.read_head();
        self.install_commit();
        self.lh.n = 0;
        self.write_head();
    }

    fn commit(&mut self) {
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

fn log_begin() {
    debug!("{:?} log_begin", std::thread::current().id());
    let mut log_guard = unsafe {LOG.lock.lock().unwrap()};
    loop {
        if unsafe {LOG.committing} {
            log_guard = self.1.wait(log_guard).unwrap();
        }
        else if (log_guard.lh.n + (log_guard.outstanding + 1) * MAXOPBLOCKS) > log_guard.size {
            // this transaction might exhaust log space;
            log_guard = self.1.wait(log_guard).unwrap();
        } else {
            log_guard.outstanding += 1;
            break;
        }
    }
}

fn log_end(&self) {
    debug!("{:?} log_end", std::thread::current().id());
    let mut log_guard = self.0.lock().unwrap();
    assert!(log_guard.outstanding > 0);
    log_guard.outstanding -= 1;
    assert_ne!(log_guard.committing, true);
    if log_guard.outstanding == 0 {
        log_guard.committing = true;
    }
    log_guard.commit();
    log_guard.committing = false;
    self.1.notify_all();
}

fn write(&mut self, buffer: RwLockWriteGuard<BufferBlock>) {
    let mut log_guard = self.0.lock().unwrap();
    assert!(log_guard.lh.n < LOGSIZE as u32);
    assert!(log_guard.outstanding > 0);
    let mut n = log_guard.lh.n;

    // log absorption
    n = (0..log_guard.lh.n)
        .find(|i| log_guard.lh.block[*i as usize] == buffer.id())
        .unwrap_or(n);
    

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


// the api of logging layer
pub fn log_write<T, V>(
    mut buffer: RwLockWriteGuard<BufferBlock>,
    offset: usize,
    f: impl FnOnce(&mut T) -> V,
) -> V {
    log_begin();
    let ret = buffer.write(offset, f);
    write(buffer);
    log_end();
    ret
}

pub fn log_write<T, V>(
    buffer: RwLockWriteGuard<BufferBlock>,
    offset: usize,
    f: impl FnOnce(&mut T) -> V,
) -> V {
    unsafe { LOG_MANAGER.log_write(buffer, offset, f) }
}

#[cfg(test)]
mod test {
    use std::{
        fs::{File, OpenOptions},
        io::Write,
        sync::Arc,
        thread,
    };

    use env_logger::{Builder, Target};

    use super::super::filedisk::FileDisk;
    use super::*;

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
    fn test_mul_thread() {
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
        // builder
        Builder::new()
            .target(Target::Stdout)
            .is_test(true)
            .filter_level(log::LevelFilter::Debug)
            .init();
        sb.logstart = 2;
        sb.nlog = LOGSIZE;
        let mut log = Log::new();
        log.init(&sb, filedisk.clone());
        let mut lh = LogHeader::new();
        lh.n = 0;
        log.lh = lh;
        log.write_head();
        unsafe { LOG_MANAGER.init(&sb, filedisk.clone()) };
        let mut handles = Vec::new();
        for i in 0..100 as u8 {
            let filedisk = filedisk.clone();
            let handle = thread::spawn(move || {
                log_write(
                        get_buffer_block(i as u32 + 3 + LOGSIZE, filedisk.clone())
                        .write()
                        .unwrap(),
                    0,
                    |b: &mut u8| {
                        *b = i;
                    },
                );
            });
            handles.push(handle);
        }

        handles
            .into_iter()
            .for_each(|handle| handle.join().unwrap());

        for i in 0..100u8 {
            let _ = get_buffer_block(i as u32 + 3 + LOGSIZE, filedisk.clone())
                .read()
                .unwrap()
                .read(0, |b: &u8| {
                    assert_eq!(*b, i);
                });
        }
    }
}
