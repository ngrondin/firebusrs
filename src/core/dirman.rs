use std::{cell::UnsafeCell, sync::{Arc, Mutex}};

use super::rcu::{RCUMap, RCUValue};

struct FunctionInfo {
    name: String
}

impl FunctionInfo {
    fn new(name: &str) -> Self {
        Self {name: String::from(name)}
    }
}

impl Clone for FunctionInfo {
    fn clone(&self) -> Self {
        Self { name: self.name.clone() }
    }
}


struct NodeInfo {
    id: u32,
    functions: Arc<RCUValue<Vec<FunctionInfo>>>
}

impl NodeInfo {
    fn new(id: u32) -> Self {
        Self {id, functions: Arc::new(RCUValue::new(Vec::new()))}
    }

    fn get_function_info(&self, name: &str) -> Option<&FunctionInfo> {
        let rv = self.functions.read();
        for function_info in rv {
            if function_info.name == name {
                return Some(function_info);
            }
        }
        None
    }
}

impl Clone for NodeInfo {
    fn clone(&self) -> Self {
        Self { 
            id: self.id.clone(), 
            functions: self.functions.clone()
        }
    }
}


pub struct DirManager {
    nodes: Arc<RCUValue<Vec<NodeInfo>>>
}

impl DirManager {
    pub fn new() -> Self {
        Self { 
            nodes: Arc::new( RCUValue::new( Vec::new()))
        }
    }

    fn get_node_info(&self, node_id: u32) -> Option<&NodeInfo> {
        let rv = self.nodes.read();
        for node_info in rv {
            if node_info.id == node_id {
                return Some(node_info);
            }
        }
        None
    }

    pub fn find_best_function(&self, name: &str) -> Option<u32> {
        let rv = self.nodes.read();
        for node_info in rv {
            for fi in node_info.functions.read() {
                if fi.name == name {
                    return Some(node_info.id);
                }
            }
        }
        return None;
    }

    pub fn add_node(&self, node_id:u32) {
        match self.get_node_info(node_id) {
            Some(_node_info) => (),
            None => {
                self.nodes.set(|val| {
                    let node_info = NodeInfo::new(node_id);
                    val.push(node_info.clone());
                });
            }
        }
    }

    pub fn add_function(&self, name: &str, node_id: u32) {
        self.add_node(node_id);
        let node_info = self.get_node_info(node_id).unwrap();
        match node_info.get_function_info(name) {
            Some(_function_info) => (),
            None => {
                node_info.functions.set(|val| {
                    let function_info = FunctionInfo::new(name);
                    val.push(function_info.clone());
                });
            }
        }
    }


}