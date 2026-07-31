use std::collections::VecDeque;
use std::sync::Mutex;

const DEFAULT_CAP: usize = 500;

pub struct LogBuffer {
    inner: Mutex<VecDeque<String>>,
    cap: usize,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(DEFAULT_CAP)),
            cap: DEFAULT_CAP,
        }
    }

    pub fn push(&self, line: impl Into<String>) {
        let mut guard = self.inner.lock().expect("log buffer poisoned");
        if guard.len() >= self.cap {
            guard.pop_front();
        }
        let text = line.into();
        eprintln!("[gateway] {text}");
        guard.push_back(text);
    }

    pub fn snapshot(&self, limit: usize) -> Vec<String> {
        let guard = self.inner.lock().expect("log buffer poisoned");
        let take = limit.min(guard.len());
        guard.iter().rev().take(take).cloned().collect::<Vec<_>>().into_iter().rev().collect()
    }
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new()
    }
}
