use std::{sync::{Mutex, Arc, RwLock}, vec, cell::RefCell};
use clap::builder::NonEmptyStringValueParser;
use lazy_static::*;
use super::lru::*;

pub const BLOCK_SIZE : usize = 512;
pub const BLOCK_NUM : usize = 64;
pub const SHARD_NUM : usize = 16;

pub trait BlockDevice : Send + Sync {
    fn read_block(&self, block_id: usize, buf: &mut [u8]);
    fn write_block(&self, block_id: usize, buf: &[u8]);
}

pub struct BufferLayer {
    buffer_pool: Vec<Arc<RwLock<BufferBlock>>>,
    
    // LRU, manage the buffer pool
    // lru: HandleTable,
}

impl BufferLayer {
    pub fn new() -> Self {
        let mut buffer_pool = Vec::with_capacity(BLOCK_NUM);
        for _ in 0..BLOCK_NUM {
            buffer_pool.push(Arc::new(RwLock::new(BufferBlock::new())));
        }
        Self {
            buffer_pool: buffer_pool,
            // lru: HandleTable::new(BLOCK_NUM, SHARD_NUM),
        }
    }
}

pub struct BufferBlock {
    dirty: bool,
    block_id: usize,
    block_device: Option<Arc<dyn BlockDevice>>,
    data: Vec<u8>,
}

impl BufferBlock {
    pub fn new() -> Self {
        Self {
            dirty: false,
            block_id: 0,
            block_device: None,
            data: vec![0; BLOCK_SIZE],
        }
    }

    pub fn init_block(block_id: usize, block_device: Arc<dyn BlockDevice>) -> Self {
        let mut data = vec![0; BLOCK_SIZE];
        block_device.read_block(block_id, data.as_mut());
        Self {
            dirty: false,
            block_id: block_id,
            block_device: Some(block_device),
            data: data,
        }
    }

    pub fn sync(&mut self) {
        if self.dirty {
            self.dirty = false;
            self.block_device.as_ref().unwrap().write_block(self.block_id, &self.data);
        }
    }

    fn addr_of_offset(&self, offset: usize) -> usize {
        &self.data[offset] as *const u8 as usize
    }
    
    pub fn get_ref<T>(&self, offset: usize) -> &T where T: Sized {
        let type_size = core::mem::size_of::<T>();
        assert!(offset + type_size <= BLOCK_SIZE);
        let addr = self.addr_of_offset(offset);
        unsafe { &*(addr as *const T) }
    }

    pub fn get_mut<T>(&mut self, offset: usize) -> &mut T where T: Sized {
        let type_size = core::mem::size_of::<T>();
        assert!(offset + type_size <= BLOCK_SIZE);
        self.dirty = true;
        let addr = self.addr_of_offset(offset);
        unsafe { &mut *(addr as *mut T) }
    }
}

// test
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buf_data_size() {
        assert_eq!(std::mem::size_of::<BufferBlock>(), BLOCK_SIZE);
    }

    #[test]
    fn test_vec_copy() {
    }

}