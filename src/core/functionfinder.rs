use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::dirman::FunctionInfo;
use super::message::MessageType;
use super::nodecore::NodeCore;

// Mirrors io.firebus.FunctionFinder.
pub struct FunctionFinder {
    node_core: Arc<NodeCore>,
    function_name: String,
    list: Vec<Arc<FunctionInfo>>,
    try_pointer: usize,
    wait_before_broadcast_ms: i64
}

impl FunctionFinder {
    pub const SUB_TIMEOUT_MS: i32 = 500;

    pub fn new(node_core: Arc<NodeCore>, function_name: &str) -> Self {
        let mut finder = Self {
            node_core,
            function_name: String::from(function_name),
            list: Vec::new(),
            try_pointer: 0,
            wait_before_broadcast_ms: 0
        };
        finder.refresh_list();
        finder
    }

    // Mirrors refreshList: collect every known provider of the function,
    // ordered by descending combined rating.
    fn refresh_list(&mut self) {
        let dir = self.node_core.get_directory();
        let count = dir.get_node_count();
        let mut list: Vec<Arc<FunctionInfo>> = Vec::new();
        for i in 0..count {
            if let Some(ni) = dir.get_node(i) {
                if let Some(fi) = ni.get_function_information(&self.function_name) {
                    let combined = fi.get_combined_rating();
                    let pos = list.iter().position(|x| x.get_combined_rating() < combined).unwrap_or(list.len());
                    list.insert(pos, fi);
                }
            }
        }
        self.list = list;
        self.try_pointer = 0;
    }

    // Mirrors findMore: broadcast a GetFunctionInfo to all nodes, then refresh.
    fn find_more(&mut self) {
        eprintln!("fb.funcfinder.broadcasting");
        if self.wait_before_broadcast_ms > 0 {
            thread::sleep(Duration::from_millis(self.wait_before_broadcast_ms as u64));
        }
        let find_msg = self.node_core.message_generator.new_message(
            0,
            MessageType::GetFunctionInfo,
            &self.function_name,
            Arc::new(super::payload::Payload::empty())
        );
        // sendAndWait(findMsg, subTimeout)
        let corrman = self.node_core.get_correlation_manager();
        let _ = corrman.send_and_wait(find_msg, Self::SUB_TIMEOUT_MS);
        self.wait_before_broadcast_ms += 1000;
        self.refresh_list();
    }

    // Mirrors findNext.
    pub fn find_next(&mut self) -> Option<Arc<FunctionInfo>> {
        if self.try_pointer >= self.list.len() {
            self.find_more();
        }
        if self.try_pointer < self.list.len() {
            let fi = self.list[self.try_pointer].clone();
            self.try_pointer += 1;
            Some(fi)
        } else {
            eprintln!(
                "fb.funcfinder.notfound function={} listlen={} pointer={}",
                self.function_name,
                self.list.len(),
                self.try_pointer
            );
            None
        }
    }
}
