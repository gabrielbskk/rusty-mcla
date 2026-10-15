//! Hash → file-name dictionary. Names are recovered offline (strings from the
//! executable, references inside data files) and shipped as a plain text list.

use crate::hash::joaat;
use hashbrown::HashMap;

#[derive(Default, Clone)]
pub struct Names {
    map: HashMap<u32, String>,
}

impl Names {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, name: &str) {
        let n = name.trim();
        if n.is_empty() {
            return;
        }
        self.map.entry(joaat(n)).or_insert_with(|| n.to_string());
    }

    /// Load one name per line (comments start with `#`).
    pub fn load_list(&mut self, text: &str) {
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            self.add(l);
        }
    }

    pub fn try_get(&self, hash: u32) -> Option<&str> {
        self.map.get(&hash).map(|s| s.as_str())
    }

    pub fn get(&self, hash: u32) -> String {
        match self.map.get(&hash) {
            Some(s) => s.clone(),
            None => format!("0x{:08x}", hash),
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}
