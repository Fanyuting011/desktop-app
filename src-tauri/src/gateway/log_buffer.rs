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
        guard
            .iter()
            .rev()
            .take(take)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    pub fn snapshot_filtered(
        &self,
        limit: usize,
        profile_id: &str,
        profile_name: Option<&str>,
    ) -> Vec<String> {
        let guard = self.inner.lock().expect("log buffer poisoned");
        let name_tag = profile_name.map(|name| format!("[{name}]"));
        let filtered = guard.iter().filter(|line| {
            line.contains(profile_id)
                || name_tag
                    .as_ref()
                    .map(|tag| line.contains(tag))
                    .unwrap_or(false)
        });
        let matching: Vec<_> = filtered.cloned().collect();
        let take = limit.min(matching.len());
        matching
            .into_iter()
            .rev()
            .take(take)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtered_snapshot_matches_profile_name_tag_or_id() {
        let buffer = LogBuffer::new();
        buffer.push("全局消息");
        buffer.push("[生产机] 隧道已建立");
        buffer.push("配置 profile-1 连接失败");
        buffer.push("[测试机] 隧道已建立");

        assert_eq!(
            buffer.snapshot_filtered(10, "profile-1", Some("生产机")),
            vec![
                "[生产机] 隧道已建立".to_string(),
                "配置 profile-1 连接失败".to_string(),
            ]
        );
    }

    #[test]
    fn filtered_snapshot_applies_limit_after_filtering() {
        let buffer = LogBuffer::new();
        buffer.push("[生产机] 第一条");
        buffer.push("[测试机] 不匹配");
        buffer.push("[生产机] 第二条");

        assert_eq!(
            buffer.snapshot_filtered(1, "profile-1", Some("生产机")),
            vec!["[生产机] 第二条".to_string()]
        );
    }
}
