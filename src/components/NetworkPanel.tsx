import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface GatewayProfileLite {
  id: string;
  name: string;
  host: string;
}

interface NetworkLogEntry {
  id: string;
  tsMs: number;
  profileId: string;
  protocol: string;
  target: string;
  ok: boolean;
  error: string | null;
}

interface NetworkPanelProps {
  active: boolean;
  profiles: GatewayProfileLite[];
}

const POLL_MS = 2000;
const LOG_LIMIT = 200;

function formatTime(ms: number) {
  const d = new Date(ms);
  return d.toLocaleTimeString("zh-CN", { hour12: false });
}

export default function NetworkPanel({ active, profiles }: NetworkPanelProps) {
  const [filter, setFilter] = useState("all");
  const [rows, setRows] = useState<NetworkLogEntry[]>([]);
  const [error, setError] = useState("");

  const nameOf = useCallback(
    (id: string) => profiles.find((p) => p.id === id)?.name || profiles.find((p) => p.id === id)?.host || id,
    [profiles],
  );

  const load = useCallback(async () => {
    try {
      const entries = await invoke<NetworkLogEntry[]>("gateway_get_network_logs", {
        profileId: filter === "all" ? null : filter,
        limit: LOG_LIMIT,
      });
      setRows(entries);
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }, [filter]);

  useEffect(() => {
    if (!active) return;
    load();
    const id = window.setInterval(load, POLL_MS);
    return () => window.clearInterval(id);
  }, [active, load]);

  async function clear() {
    try {
      await invoke("gateway_clear_network_logs", {
        profileId: filter === "all" ? null : filter,
      });
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <section className="network-page">
      <div className="page-head">
        <h2>Network</h2>
        <div className="page-actions">
          <select
            className="filter-select"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          >
            <option value="all">全部主机</option>
            {profiles.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name || p.host || p.id}
              </option>
            ))}
          </select>
          <button type="button" className="btn ghost" onClick={clear}>
            Clear
          </button>
        </div>
      </div>

      <div className="network-table-wrap scroll">
        <table className="network-table">
          <thead>
            <tr>
              <th>Time</th>
              <th>Host</th>
              <th>Proto</th>
              <th>Target</th>
              <th>Result</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.id}>
                <td>{formatTime(r.tsMs)}</td>
                <td>{nameOf(r.profileId)}</td>
                <td className="mono">{r.protocol}</td>
                <td className="mono">{r.target}</td>
                <td>
                  {r.ok ? (
                    <span className="tag ok">OK</span>
                  ) : (
                    <span className="tag err" title={r.error ?? ""}>
                      Fail
                    </span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {rows.length === 0 && <p className="empty">暂无记录</p>}
      </div>
      {error && <p className="banner">{error}</p>}
    </section>
  );
}
