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
        // Hold a low, well-within-range port pair busy with real listeners (rather than
        // relying on an OS-assigned ephemeral port, which can land near the u16 range ceiling
        // and make the allocator's bounded search flaky) and confirm the allocator steps past
        // both of them. Try a few candidate bases in case the default pair is already occupied
        // by another process on this machine (e.g. a running dev build of this app).
        let candidates = [17890u16, 27890, 37890, 47890, 57890];
        let bound = candidates.into_iter().find_map(|base| {
            let http = TcpListener::bind(("127.0.0.1", base)).ok()?;
            let socks = TcpListener::bind(("127.0.0.1", base + 1)).ok()?;
            Some((base, http, socks))
        });
        let (base, _http_listener, _socks_listener) =
            bound.expect("could not find a free port pair to hold busy for the test");

        let used = HashSet::new();
        let (h, s) = allocate_port_pair(&used, base).unwrap();
        assert!(h != base && s != base);
        assert!(h != base + 1 && s != base + 1);
    }
}
