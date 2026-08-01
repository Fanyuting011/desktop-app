use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Mutex;

const CAP: usize = 1000;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkLogEntry {
    pub id: String,
    pub ts_ms: u64,
    pub profile_id: String,
    pub protocol: String,
    pub target: String,
    pub ok: bool,
    pub error: Option<String>,
}

pub struct NetworkLogBuffer {
    inner: Mutex<VecDeque<NetworkLogEntry>>,
}

impl NetworkLogBuffer {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(CAP)),
        }
    }

    pub fn push(&self, entry: NetworkLogEntry) {
        let mut guard = self.inner.lock().expect("network log buffer poisoned");
        if guard.len() >= CAP {
            guard.pop_front();
        }
        guard.push_back(entry);
    }

    pub fn snapshot(&self, profile_id: Option<&str>, limit: usize) -> Vec<NetworkLogEntry> {
        let guard = self.inner.lock().expect("network log buffer poisoned");
        let filtered: Vec<_> = guard
            .iter()
            .filter(|e| profile_id.map(|p| e.profile_id == p).unwrap_or(true))
            .cloned()
            .collect();
        let take = limit.min(filtered.len());
        filtered
            .iter()
            .rev()
            .take(take)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    pub fn clear(&self, profile_id: Option<&str>) {
        let mut guard = self.inner.lock().expect("network log buffer poisoned");
        match profile_id {
            None => guard.clear(),
            Some(pid) => guard.retain(|e| e.profile_id != pid),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, profile: &str) -> NetworkLogEntry {
        NetworkLogEntry {
            id: id.into(),
            ts_ms: 0,
            profile_id: profile.into(),
            protocol: "http".into(),
            target: "example.com".into(),
            ok: true,
            error: None,
        }
    }

    #[test]
    fn push_and_snapshot_returns_entries_in_order() {
        let buf = NetworkLogBuffer::new();
        buf.push(entry("1", "p1"));
        buf.push(entry("2", "p1"));
        let snap = buf.snapshot(None, 10);
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].id, "1");
        assert_eq!(snap[1].id, "2");
    }

    #[test]
    fn snapshot_filters_by_profile_id() {
        let buf = NetworkLogBuffer::new();
        buf.push(entry("1", "p1"));
        buf.push(entry("2", "p2"));
        buf.push(entry("3", "p1"));
        let snap = buf.snapshot(Some("p1"), 10);
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].id, "1");
        assert_eq!(snap[1].id, "3");
    }

    #[test]
    fn cap_evicts_oldest_when_full() {
        let buf = NetworkLogBuffer::new();
        for i in 0..1001 {
            buf.push(entry(&i.to_string(), "p1"));
        }
        let snap = buf.snapshot(None, 2000);
        assert_eq!(snap.len(), 1000);
        assert_eq!(snap[0].id, "1");
        assert_eq!(snap[999].id, "1000");
    }

    #[test]
    fn clear_all() {
        let buf = NetworkLogBuffer::new();
        buf.push(entry("1", "p1"));
        buf.clear(None);
        assert!(buf.snapshot(None, 10).is_empty());
    }

    #[test]
    fn clear_by_profile_id() {
        let buf = NetworkLogBuffer::new();
        buf.push(entry("1", "p1"));
        buf.push(entry("2", "p2"));
        buf.clear(Some("p1"));
        let snap = buf.snapshot(None, 10);
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].id, "2");
    }
}
