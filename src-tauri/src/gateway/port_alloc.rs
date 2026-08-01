use std::collections::HashSet;
use std::net::TcpListener;

const MAX_TRIES: u16 = 200;

pub fn allocate_port_pair(used: &HashSet<u16>, base_http: u16) -> Result<(u16, u16), String> {
    let mut http = if base_http % 2 == 0 {
        base_http
    } else {
        base_http + 1
    };
    for _ in 0..MAX_TRIES {
        let socks = http + 1;
        if !used.contains(&http)
            && !used.contains(&socks)
            && port_free(http)
            && port_free(socks)
        {
            return Ok((http, socks));
        }
        http = http.saturating_add(2);
        if http > 60000 {
            break;
        }
    }
    Err("无法分配本地代理端口".into())
}

fn port_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::net::TcpListener;

    #[test]
    fn skips_ports_in_used_set() {
        let mut used = HashSet::new();
        used.insert(17890);
        used.insert(17891);
        let (h, s) = allocate_port_pair(&used, 17890).unwrap();
        assert_ne!(h, 17890);
        assert_eq!(s, h + 1);
        assert!(!used.contains(&h));
    }

    #[test]
    fn skips_ports_already_bound() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let busy = listener.local_addr().unwrap().port();
        // Force allocator to consider a range that includes busy by using busy as base when even
        let base = if busy % 2 == 0 {
            busy
        } else {
            busy.saturating_sub(1)
        };
        let mut used = HashSet::new();
        let (h, s) = allocate_port_pair(&used, base).unwrap();
        assert!(h != busy && s != busy);
    }
}
