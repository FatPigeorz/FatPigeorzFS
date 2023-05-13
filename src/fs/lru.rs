use std::{sync::{Mutex, Arc}, collections::HashMap};
use super::linkedlist::*;


struct LRUHandle {
    map: HashMap<usize, usize>,
    list : LinkedList<usize>,
}

impl LRUHandle {
    pub fn new() -> Self {
        Self { 
            map: HashMap::new(),
            list: LinkedList::new(),
        }
    }
}

struct HandleTable {
    handles: Vec<Arc<Mutex<LRUHandle>>>,
}


impl HandleTable {
    pub fn new(buffer_num:usize, shard_num:usize) -> Self {
        assert!(buffer_num % shard_num == 0);
        let mut handles: Vec<Arc<Mutex<LRUHandle>>> = Vec::with_capacity(shard_num);
        for _ in 0..shard_num {
            handles.push(Arc::new(Mutex::new(LRUHandle::new())));
        }

        // init the 0th shard
        for i in 0..buffer_num {
            let mut handle = handles[0].lock().unwrap();
            handle.map.insert(i, i);
            handle.list.push_back(i);
        }
        
        Self {
            handles: handles,
        }
    }
}


#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_new() {
        let handle_table = HandleTable::new(8, 4);
        assert_eq!(handle_table.handles.len(), 4);
        // iter the 0th shard
        let handle = handle_table.handles[0].lock().unwrap();
        let mut iter = handle.list.iter();
        assert_eq!(iter.next(), Some(&0));
        assert_eq!(iter.next(), Some(&1));
        assert_eq!(iter.next(), Some(&2));
        assert_eq!(iter.next(), Some(&3));
        assert_eq!(iter.next(), Some(&4));
        assert_eq!(iter.next(), Some(&5));
        assert_eq!(iter.next(), Some(&6));
        assert_eq!(iter.next(), Some(&7));
        assert_eq!(iter.next(), None);
    }
}