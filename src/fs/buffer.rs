use std::{sync::{Mutex, Arc, RwLock}, vec, ptr::NonNull, collections::HashMap, marker::PhantomData, fmt::{Debug, Formatter}};
use log::{debug, error, log_enabled, info, Level};


pub const BLOCK_SIZE : usize = 512;
pub const BLOCK_NUM : usize = 64;
pub const SHARD_NUM : usize = 4;

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
        // log sync
        if self.dirty {
            self.dirty = false;
            self.block_device.as_ref().unwrap().write_block(self.block_id, &self.data);
        }
    }
    
    fn offset_addr(&self, offset: usize) -> usize {
        &self.data[offset] as *const u8 as usize
    }
    
    pub fn as_ref<T>(&self, offset: usize) -> &T where T: Sized {
        let type_size = core::mem::size_of::<T>();
        assert!(offset + type_size <= BLOCK_SIZE);
        let addr = self.offset_addr(offset);
        unsafe { &*(addr as *const T) }
    }

    pub fn as_mut<T>(&mut self, offset: usize) -> &mut T where T: Sized {
        let type_size = core::mem::size_of::<T>();
        assert!(offset + type_size <= BLOCK_SIZE);
        self.dirty = true;
        let addr = self.offset_addr(offset);
        unsafe { &mut *(addr as *mut T) }
    }
}

impl Drop for BufferBlock {
    fn drop(&mut self) {
        self.sync();
    }
}

struct Node {
    data: Arc<RwLock<BufferBlock>>,
    next: Option<NonNull<Node>>,
    prev: Option<NonNull<Node>>,
}

type NodePtr = NonNull<Node>;

struct LruHandle {
    map: HashMap<usize, NodePtr>,
    head: Option<NodePtr>,
    tail: Option<NodePtr>,
    marker: PhantomData<Node>,
}

unsafe impl Send for LruHandle {}
unsafe impl Sync for LruHandle {}

impl LruHandle {
    pub fn new() -> Self {
        // dummy head and dummy tail
        unsafe {
            let mut head = NonNull::new_unchecked(Box::leak(Box::new(Node {
                data: Arc::new(RwLock::new(BufferBlock::new())),
                next: None,
                prev: None,
            })));
            let mut tail = NonNull::new_unchecked(Box::leak(Box::new(Node {
                data: Arc::new(RwLock::new(BufferBlock::new())),
                next: None,
                prev: None,
            })));
            head.as_mut().next = Some(tail);
            tail.as_mut().prev = Some(head);
            Self {
                map: HashMap::new(),
                head: Some(head),
                tail: Some(tail),
                marker: PhantomData,
            }
        }
    }

    pub fn get(&mut self, block_id : &usize, block_device: Arc<dyn BlockDevice>)  -> Option<Arc<RwLock<BufferBlock>>> {
        // print block_id
        if let Some(node) = self.map.get(block_id) {
            // buffer hit!
            let node = unsafe { NonNull::new_unchecked(Box::leak(self.unlink_node(*node))) };
            self.push_back(node);
            unsafe {Some(node.as_ref().data.clone())}
        } else {
            unsafe {
                let mut cursor = self.head.unwrap().as_mut().next;
                while let Some(mut node) = cursor.unwrap().as_mut().next {
                    node = cursor.unwrap();
                    if Arc::strong_count(&node.as_ref().data) == 1 {
                        self.map.remove(&node.as_ref().data.read().unwrap().block_id);
                        let _ = self.unlink_node(node);
                        let new_node = NodePtr::new(Box::into_raw(Box::new(Node {
                            data: Arc::new(RwLock::new(BufferBlock::init_block(*block_id, block_device))),
                            next: None,
                            prev: None,
                        }))).unwrap();
                        self.push_back(new_node);
                        self.map.insert(*block_id, new_node);
                        return Some(new_node.as_ref().data.clone()); 
                    }
                    cursor = node.as_mut().next;
                }
                None
            }
            }
        }
    
    #[inline]
    fn push_front(&self, mut node: NonNull<Node>) {
        unsafe {
            node.as_mut().next = self.head.unwrap().as_mut().next;
            node.as_mut().prev = Some(self.head.unwrap());
            self.head.unwrap().as_mut().next.unwrap().as_mut().prev = Some(node);
            self.head.unwrap().as_mut().next = Some(node);
        }
    }
    
    #[inline]
    fn push_back(&self, mut node: NonNull<Node>) {
        unsafe {
            node.as_mut().prev = self.tail.unwrap().as_mut().prev;
            node.as_mut().next = Some(self.tail.unwrap());
            self.tail.unwrap().as_mut().prev.unwrap().as_mut().next = Some(node);
            self.tail.unwrap().as_mut().prev = Some(node);
        }
    }
    
    #[inline]
    fn unlink_node(&self, mut node: NonNull<Node>) -> Box<Node> {
        unsafe {
            node.as_mut().prev.unwrap().as_mut().next = node.as_mut().next;
            node.as_mut().next.unwrap().as_mut().prev = node.as_mut().prev;
            node.as_mut().prev = None;
            node.as_mut().next = None;
            Box::from_raw(node.as_ptr())
        }
    }
}
    
impl Debug for LruHandle {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        unsafe {
            let mut cursor = self.head.unwrap().as_mut().next;
            while let Some(_) = cursor.unwrap().as_mut().next {
                let _ = write!(f, "{:?}-", cursor.unwrap().as_ref().data.read().unwrap().block_id);
                cursor = Some(cursor.unwrap().as_mut().next.unwrap());
            }
        }
        Ok(())
    }
}

pub struct HandleTable {
    handles : Vec<Arc<Mutex<LruHandle>>>,
}

impl HandleTable {
    pub fn new(shard_num: usize, block_num: usize) -> Self {
        assert_eq!(block_num % shard_num, 0);
        let mut handles = Vec::with_capacity(shard_num);
        for _ in 0..shard_num {
            let handle = LruHandle::new();
            // push block_num / shard_num nodes
            for _ in 0..(block_num / shard_num) {
                let node = NodePtr::new(Box::into_raw(Box::new(Node {
                    data: Arc::new(RwLock::new(BufferBlock::new())),
                    next: None,
                    prev: None,
                }))).unwrap();
                handle.push_front(node);
            }
            handles.push(Arc::new(Mutex::new(handle)));
        }
        Self {
            handles: handles,
        }
    }
    
    pub fn get(&mut self, block_id: &usize, block_device: Arc<dyn BlockDevice>) -> Arc<RwLock<BufferBlock>> {
        let shard_id = block_id % SHARD_NUM;
        // continue get until the block is in the buffer pool
        loop {
            let mut handle = self.handles[shard_id].lock().unwrap();
            if let Some(block) = handle.get(block_id, block_device.clone()) {
                return block;
            }
        }
    }
}

pub trait BlockDevice : Send + Sync {
    fn read_block(&self, block_id: usize, buf: &mut [u8]);
    fn write_block(&self, block_id: usize, buf: &[u8]);
}

use once_cell::sync::Lazy;
static mut BUFFER_LAYER: Lazy<HandleTable> = Lazy::new(|| HandleTable::new(SHARD_NUM, BLOCK_NUM));

pub fn buffer_read(
    block_id: usize,
    block_device: Arc<dyn BlockDevice>
) -> Arc<RwLock<BufferBlock>> {
    unsafe { BUFFER_LAYER.get(&block_id, block_device).clone() }
}

// test
#[cfg(test)]
mod tests {
    use std::{fs::{OpenOptions, File}, io::Write, thread};

    use super::*;
    #[test]
    fn test_lru() {
        let lru = LruHandle::new();
        let node1 = NodePtr::new(Box::into_raw(Box::new(Node {
            data: Arc::new(RwLock::new(BufferBlock::new())),
            next: None,
            prev: None,
        }))).unwrap();
        unsafe { node1.as_ref().data.write().unwrap().block_id = 0 };
        let node2 = NodePtr::new(Box::into_raw(Box::new(Node {
            data: Arc::new(RwLock::new(BufferBlock::new())),
            next: None,
            prev: None,
        }))).unwrap();
        unsafe { node2.as_ref().data.write().unwrap().block_id = 1 };
        let node3 = NodePtr::new(Box::into_raw(Box::new(Node {
            data: Arc::new(RwLock::new(BufferBlock::new())),
            next: None,
            prev: None,
        }))).unwrap();
        unsafe { node3.as_ref().data.write().unwrap().block_id = 2 };
        lru.push_front(node1);
        lru.push_front(node2);
        lru.push_front(node3);
        unsafe {
            assert_eq!(lru.head.unwrap().as_ref().next.unwrap().as_ref().data.read().unwrap().block_id, 2);
            assert_eq!(lru.head.unwrap().as_ref().next.unwrap().as_ref().next.unwrap().as_ref().data.read().unwrap().block_id, 1);
            assert_eq!(lru.head.unwrap().as_ref().next.unwrap().as_ref().next.unwrap().as_ref().next.unwrap().as_ref().data.read().unwrap().block_id, 0);
        }
        let another_node2 = Box::leak(lru.unlink_node(node2));
        // node 2 has been drop
        unsafe {
            assert_eq!(lru.head.unwrap().as_ref().next.unwrap().as_ref().data.read().unwrap().block_id, 2);
            assert_eq!(lru.head.unwrap().as_ref().next.unwrap().as_ref().next.unwrap().as_ref().data.read().unwrap().block_id, 0);
        }
        // push front node 2
        unsafe {
            assert_eq!(NonNull::new_unchecked(another_node2), node2);
            lru.push_front(NonNull::new_unchecked(another_node2));
            assert_eq!(lru.head.unwrap().as_ref().next.unwrap().as_ref().data.read().unwrap().block_id, 1);
            assert_eq!(lru.head.unwrap().as_ref().next.unwrap().as_ref().next.unwrap().as_ref().data.read().unwrap().block_id, 2);
            assert_eq!(lru.head.unwrap().as_ref().next.unwrap().as_ref().next.unwrap().as_ref().next.unwrap().as_ref().data.read().unwrap().block_id, 0);
        }
    }
    
    #[test]
    fn test_strong_ref_cnt() {
        let block = Arc::new(RwLock::new(BufferBlock::new()));
        let node = NodePtr::new(Box::into_raw(Box::new(Node {
            data: block,
            next: None,
            prev: None,
        }))).unwrap();

        let node2 = node.clone();
        let _ = node.clone();
        let _ = node.clone();
        let mut node5 = node.clone();
        unsafe { assert_eq!(Arc::strong_count(&node2.as_ref().data), 1); }
        
        let _data  = unsafe { node5.as_mut().data.clone() };

        // get_ref_cnt
        unsafe { assert_eq!(Arc::strong_count(&node2.as_ref().data), 2); }
    }
    
    #[test]
    fn test_get() {
        let mut table = HandleTable::new(SHARD_NUM, BLOCK_NUM);
        use super::super::filedisk::FileDisk;
        let file: File= OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open("./test.img")
            .unwrap();
        file.set_len(1024 * 1024).unwrap();
        let file_disk = Arc::new(FileDisk::new(file));

        // loop write
        for i in 0..64 {
            file_disk.write_block(i, &[i as u8; 512]);
        }
        // loop test
        for i in 0..640 {
            let buffer = table.get(&(i % 64), file_disk.clone());
            assert_eq!(buffer.read().unwrap().data, [(i % 64) as u8; 512]);
        }
    }
    
    #[test]
    fn test_drop() {
        use super::super::filedisk::FileDisk;
        let mut file: File= OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open("./test.img")
            .unwrap();
        file.set_len(1024 * 1024).unwrap();
        file.write_all(&[0 as u8; 1024*1024]).unwrap();
        let filedisk = Arc::new(FileDisk::new(file));

        // loop write
        for i in 0..64 {
            filedisk.write_block(i, &[i as u8; 512]);
        }

        // get buffer
        let mut table = HandleTable::new(SHARD_NUM, BLOCK_NUM);
        for i in 0..32 {
            let buffer = table.get(&((i * 4) % 64), filedisk.clone());
            assert_eq!(Arc::strong_count(&buffer), 2);
            assert_eq!(buffer.read().unwrap().data, [((i * 4) % 64) as u8; 512]);
        }
    }
    
    #[test]
    fn test_layer() {
        use super::super::filedisk::FileDisk;
        let mut file: File= OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open("./test.img")
            .unwrap();
        file.set_len(1024 * 1024).unwrap();
        file.write_all(&[0 as u8; 1024*1024]).unwrap();
        let filedisk = Arc::new(FileDisk::new(file));
        for i in 0..64 {
            filedisk.write_block(i, &[64 - i as u8; 512]);
        }

        // 64 thread
        let mut handles = Vec::new();
        for _ in 0..64 {
            let filedisk = filedisk.clone();
            let handle = thread::spawn(move || {
                for j in 0..64 {
                    let buffer = buffer_read(j, filedisk.clone());
                    assert_eq!(buffer.clone().read().unwrap().data, [(64 - j) as u8; 512]);
                }
            });
            handles.push(handle);
        }
        handles.into_iter().for_each(|handle| handle.join().unwrap());
    }
}
