/// Map proxy dial errors to (category, hint). No active probing.
pub fn classify_network_error(error: Option<&str>) -> (String, Option<String>) {
    let Some(raw) = error else {
        return ("ok".into(), None);
    };
    let lower = raw.to_lowercase();

    let (category, hint) = if raw.contains("上游")
        || lower.contains("upstream")
        || lower.contains("socks5 握手")
        || lower.contains("socks5 connect")
    {
        (
            "upstream",
            "上游代理不可达或拒绝。请确认 Clash 等已开启，且应用里上游地址正确。",
        )
    } else if lower.contains("lookup")
        || lower.contains("name or service not known")
        || lower.contains("nodename nor servname")
        || lower.contains("no such host")
    {
        (
            "dns",
            "域名解析失败。检查 DNS，或改用 IP；若仅远程解析失败，查看服务器 DNS。",
        )
    } else if lower.contains("timed out") || lower.contains("timeout") {
        (
            "timeout",
            "连接超时。目标慢、链路不稳或被干扰；可重试，并检查上游是否正常。",
        )
    } else if lower.contains("connection refused") || lower.contains("actively refused") {
        (
            "refused",
            "连接被拒绝。对端未监听该端口，或地址/端口写错。",
        )
    } else if lower.contains("broken pipe")
        || lower.contains("tunnel")
        || raw.contains("隧道")
    {
        (
            "tunnel",
            "隧道或本地代理可能已断开。请回到 Hosts 重新 Connect。",
        )
    } else if lower.contains("connection reset") || lower.contains("reset by peer") {
        (
            "blocked",
            "连接被重置。可能被墙或中间设备干扰；可查看上游代理日志。",
        )
    } else {
        (
            "other",
            "请求失败。请展开原始错误，并到 Logs 查看网关/SSH 详情。",
        )
    };

    (category.into(), Some(hint.into()))
}

#[cfg(test)]
mod tests {
    use super::classify_network_error;

    #[test]
    fn ok_when_no_error() {
        assert_eq!(classify_network_error(None), ("ok".into(), None));
    }

    #[test]
    fn upstream_from_chinese_or_connect() {
        let (c, h) = classify_network_error(Some("上游 HTTP 代理 CONNECT 失败: 503"));
        assert_eq!(c, "upstream");
        let hint = h.unwrap();
        assert!(hint.contains("Clash") || hint.contains("上游"));
    }

    #[test]
    fn dns_from_lookup() {
        let (c, _) = classify_network_error(Some("failed to lookup address information"));
        assert_eq!(c, "dns");
    }

    #[test]
    fn timeout_category() {
        let (c, _) = classify_network_error(Some("connection timed out"));
        assert_eq!(c, "timeout");
    }

    #[test]
    fn refused_category() {
        let (c, _) = classify_network_error(Some("Connection refused (os error 61)"));
        assert_eq!(c, "refused");
    }

    #[test]
    fn other_fallback() {
        let (c, h) = classify_network_error(Some("weird glitch"));
        assert_eq!(c, "other");
        let hint = h.unwrap();
        assert!(hint.contains("Logs") || hint.contains("日志"));
    }
}
