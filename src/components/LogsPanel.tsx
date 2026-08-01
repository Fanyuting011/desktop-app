import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface GatewayProfileLite {
  id: string;
  name: string;
  host: string;
}

interface LogsPanelProps {
  profiles: GatewayProfileLite[];
}

const POLL_MS = 3000;
const LOG_LIMIT = 120;

export default function LogsPanel({ profiles }: LogsPanelProps) {
  const [filter, setFilter] = useState("all");
  const [logs, setLogs] = useState<string[]>([]);
  const [error, setError] = useState("");

  useEffect(() => {
    let active = true;

    async function load() {
      try {
        const lines = await invoke<string[]>("gateway_get_logs", {
          limit: LOG_LIMIT,
          profileId: filter === "all" ? null : filter,
        });
        if (active) {
          setLogs(lines);
          setError("");
        }
      } catch (e) {
        if (active) setError(String(e));
      }
    }

    load();
    const id = window.setInterval(load, POLL_MS);
    return () => {
      active = false;
      window.clearInterval(id);
    };
  }, [filter]);

  return (
    <section className="logs-page">
      <div className="page-head">
        <h2>Logs</h2>
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
      </div>
      <pre>{logs.length ? logs.join("\n") : "暂无日志"}</pre>
      {error && <p className="banner">{error}</p>}
    </section>
  );
}
