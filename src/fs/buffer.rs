use std::{sync::{Mutex, Arc, RwLock}, vec, ptr::NonNull, collections::HashMap};
use lazy_static::*;
use super::linkedlist::*;

pub const BLOCK_SIZE : usize = 512;
pub const BLOCK_NUM : usize = 64;
pub const SHARD_NUM : usize = 16;

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

// <K, V> = <block_id, buffer_block>
struct LruHandle {
    pub map: HashMap<usize, NonNull<Node<Arc<RwLock<BufferBlock>>>>>,
    pub list: LinkedList<Arc<RwLock<BufferBlock>>>,
}

impl LruHandle {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            list: LinkedList::new(),
        }
    }

    pub fn get(&mut self, block_id : usize, block_device: Arc<dyn BlockDevice>)  -> Arc<RwLock<BufferBlock>> {
        if let Some(node) = self.map.get(&block_id) {
            self.list.unlink_node(*node);
        }
        let block = Arc::new(RwLock::new(BufferBlock::init_block(block_id, block_device)));
        self.list.push_front(block.clone());
        self.map.insert(block_id, self.list.head.unwrap());
        block
    }
}
    
struct HandleTable {
    handles : Vec<Arc<Mutex<LruHandle>>>,
}

impl HandleTable {
    pub fn new(shard_num: usize, block_num: usize) -> Self {
        let mut handles = Vec::with_capacity(shard_num);
        for _ in 0..SHARD_NUM {
            handles.push(Arc::new(Mutex::new(LruHandle::new())));
        }
        Self {
            handles: handles,
        }
    }
}

pub trait BlockDevice : Send + Sync {
    fn read_block(&self, block_id: usize, buf: &mut [u8]);
    fn write_block(&self, block_id: usize, buf: &[u8]);
}

pub struct BufferLayer {
    // LRU, manage the buffer pool
    lru: HandleTable,
}

// test
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buf_data_size() {
        assert_eq!(std::mem::size_of::<BufferBlock>(), BLOCK_SIZE);
    }
}