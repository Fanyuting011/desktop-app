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
  category: string;
  hint: string | null;
}

interface NetworkPanelProps {
  active: boolean;
  profiles: GatewayProfileLite[];
  noProxySummary?: string;
}

const POLL_MS = 2000;
const LOG_LIMIT = 200;

function formatTime(ms: number) {
  const d = new Date(ms);
  return d.toLocaleTimeString("zh-CN", { hour12: false });
}

export default function NetworkPanel({ active, profiles, noProxySummary }: NetworkPanelProps) {
  const [filter, setFilter] = useState("all");
  const [failOnly, setFailOnly] = useState(false);
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

  const displayRows = failOnly ? rows.filter((row) => !row.ok) : rows;
  const latestFailId = [...displayRows].reverse().find((row) => !row.ok)?.id;

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
          <label className="network-fail-toggle">
            <input
              type="checkbox"
              checked={failOnly}
              onChange={(e) => setFailOnly(e.target.checked)}
            />
            仅失败
          </label>
          <button type="button" className="btn ghost" onClick={clear}>
            Clear
          </button>
        </div>
      </div>

      <p className="network-note">
        服务器应用经 HTTP_PROXY/ALL_PROXY 进隧道；命中 NO_PROXY 则直连、不出现在本页。
        {noProxySummary ? ` 当前 NO_PROXY：${noProxySummary}` : null}
      </p>

      <div className="network-table-wrap scroll">
        <table className="network-table">
          <thead>
            <tr>
              <th>Time</th>
              <th>Host</th>
              <th>Proto</th>
              <th>Target</th>
              <th>Category</th>
              <th>Result</th>
            </tr>
          </thead>
          <tbody>
            {displayRows.map((r) => (
              <tr
                key={r.id}
                className={r.id === latestFailId ? "network-row fail-latest" : !r.ok ? "network-row fail" : "network-row"}
              >
                <td>{formatTime(r.tsMs)}</td>
                <td>{nameOf(r.profileId)}</td>
                <td className="mono">{r.protocol}</td>
                <td className="mono">{r.target}</td>
                <td>{r.category}</td>
                <td>
                  {r.ok ? (
                    <span className="tag ok">OK</span>
                  ) : (
                    <>
                      <span className="tag err" title={`${r.hint ?? ""}\n${r.error ?? ""}`}>
                        Fail
                      </span>
                      {r.hint && <span className="network-hint">{r.hint}</span>}
                    </>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {displayRows.length === 0 && <p className="empty">暂无记录</p>}
      </div>
      {error && <p className="banner">{error}</p>}
    </section>
  );
}
