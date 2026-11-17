use std::{cell::UnsafeCell, collections::{hash_map::Iter, HashMap}, sync::{Arc, Mutex, MutexGuard}};

pub struct RCUValue<V: Clone> {
    write_val: Arc<Mutex<V>>,
    read_val: UnsafeCell<V> 
}

impl<V: Clone> RCUValue<V> {
    pub fn new(val: V) -> Self {
        Self {
            write_val: Arc::new( Mutex::new ( val.clone() ) ),
            read_val: UnsafeCell::new( val.clone() )
        }
    }

    pub fn read(&self) -> &V {
        unsafe {
            let rv = &*(self.read_val.get());
            return rv;
        }
    }

    /*pub fn lock(&self) -> MutexGuard<'_, V> {
        self.write_val.lock().unwrap()
    }*/

    /*pub fn set(&self, lock: MutexGuard<'_, V>) {
        unsafe {
            *self.read_val.get() = lock.clone();
        }
    }*/

    pub fn set<G>(&self, setter: G) where G: Fn(&mut V) {
        let mut lock = self.write_val.lock().unwrap();
        let mut new_val = lock.clone();
        setter(&mut new_val);
        *lock = new_val;
        unsafe {
            *self.read_val.get() = lock.clone();
        }
    }
}

unsafe impl<V: Clone> Sync for RCUValue<V> {
    
}


pub struct RCUMap<V: Clone> {
    write_map: Arc<Mutex<HashMap<String, V>>>,
    read_map: UnsafeCell<HashMap<String, V>>
}

impl<V: Clone> RCUMap<V> {
    pub fn new() -> Self {
        Self {
            write_map: Arc::new( Mutex::new ( HashMap::new() ) ),
            read_map: UnsafeCell::new(HashMap::new())
        }
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        unsafe {
            let rhm = &*(self.read_map.get());
            return rhm.get(key);
        }
    }

    pub fn put(&self, key: &str, value: &V) {
        let mut whm = self.write_map.lock().unwrap();
        whm.insert(String::from(key), value.clone());
        let nhm = whm.clone();
        unsafe {
            *self.read_map.get() = nhm;
        }
    }

    pub fn into_iter(&self) -> Iter<String, V> {
        unsafe {
            let rhm = &*(self.read_map.get());
            return rhm.into_iter();
        }
    }
}

unsafe impl<V: Clone> Sync for RCUMap<V> {
    
}

