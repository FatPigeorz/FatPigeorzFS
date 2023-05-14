use std::{borrow::Borrow, collections::HashMap, hash::Hash, marker::PhantomData, ptr::NonNull};

struct Node<K, V> {
    k: K,
    v: V,
    prev: Option<NonNull<Node<K, V>>>,
    next: Option<NonNull<Node<K, V>>>,
}

struct KeyRef<K, V>(NonNull<Node<K, V>>);

impl<K: Hash + Eq, V> Borrow<K> for KeyRef<K, V> {
    fn borrow(&self) -> &K {
        unsafe { &self.0.as_ref().k }
    }
}

impl<K: Hash, V> Hash for KeyRef<K, V> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        unsafe { self.0.as_ref().k.hash(state) }
    }
}

impl<K: Eq, V> PartialEq for KeyRef<K, V> {
    fn eq(&self, other: &Self) -> bool {
        unsafe { self.0.as_ref().k.eq(&other.0.as_ref().k) }
    }
}

impl<K: Eq, V> Eq for KeyRef<K, V> {}

impl<K, V> Node<K, V> {
    fn new(k: K, v: V) -> Self {
        Self {
            k,
            v,
            prev: None,
            next: None,
        }
    }
}

pub struct LruCache<K, V> {
    head: Option<NonNull<Node<K, V>>>,
    tail: Option<NonNull<Node<K, V>>>,
    map: HashMap<KeyRef<K, V>, NonNull<Node<K, V>>>,
    cap: usize,
    marker: PhantomData<Node<K, V>>,
}

impl<K: Hash + Eq + PartialEq, V> LruCache<K, V> {
    pub fn new(cap: usize) -> Self {
        Self {
            head: None,
            tail: None,
            map: HashMap::new(),
            cap,
            marker: PhantomData,
        }
    }

    pub fn put(&mut self, k: K, v: V) -> Option<V> {
        let node = Box::leak(Box::new(Node::new(k, v))).into();
        let old_node = self.map.remove(&KeyRef(node)).map(|node| {
            self.detach(node);
            node
        });
        if old_node.is_none() && self.map.len() >= self.cap {
            let tail = self.tail.unwrap();
            self.detach(tail);
            self.map.remove(&KeyRef(tail));
        }
        self.attach(node);
        self.map.insert(KeyRef(node), node);
        old_node.map(|node| unsafe {
            let node = Box::from_raw(node.as_ptr());
            node.v
        })
    }

    pub fn get(&mut self, k: &K) -> Option<&V> {
        if let Some(node) = self.map.get(k) {
            let node = *node;
            self.detach(node);
            self.attach(node);
            unsafe { Some(&node.as_ref().v) }
        } else {
            None
        }
    }

    fn detach(&mut self, mut node: NonNull<Node<K, V>>) {
        unsafe {
            match node.as_mut().prev {
                Some(mut prev) => {
                    prev.as_mut().next = node.as_ref().next;
                }
                None => {
                    self.head = node.as_ref().next;
                }
            }
            match node.as_mut().next {
                Some(mut next) => {
                    next.as_mut().prev = node.as_ref().prev;
                }
                None => {
                    self.tail = node.as_ref().prev;
                }
            }

            node.as_mut().prev = None;
            node.as_mut().next = None;
        }
    }

    fn attach(&mut self, mut node: NonNull<Node<K, V>>) {
        match self.head {
            Some(mut head) => {
                unsafe {
                    head.as_mut().prev = Some(node);
                    node.as_mut().next = Some(head);
                    node.as_mut().prev = None;
                }
                self.head = Some(node);
            }
            None => {
                unsafe {
                    node.as_mut().prev = None;
                    node.as_mut().next = None;
                }
                self.head = Some(node);
                self.tail = Some(node);
            }
        }
    }
}

impl<K, V> Drop for LruCache<K, V> {
    fn drop(&mut self) {
        while let Some(node) = self.head.take() {
            unsafe {
                self.head = node.as_ref().next;
                drop(node.as_ptr());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn it_works() {
        let mut lru = LruCache::new(3);
        assert_eq!(lru.put(1, 10), None);
        assert_eq!(lru.put(2, 20), None);
        assert_eq!(lru.put(3, 30), None);
        assert_eq!(lru.get(&1), Some(&10));
        assert_eq!(lru.put(2, 200), Some(20));
        assert_eq!(lru.put(4, 40), None);
        assert_eq!(lru.get(&2), Some(&200));
        assert_eq!(lru.get(&3), None);
    }
}