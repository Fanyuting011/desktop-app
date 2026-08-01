interface ConnectProfile {
  name: string;
  host: string;
  port: number;
}

interface ConnectingOverlayProps {
  profile: ConnectProfile;
  phase: "connecting" | "failed";
  error?: string;
  logs: string[];
  showLogs: boolean;
  onToggleLogs: () => void;
  onClose: () => void;
}

export default function ConnectingOverlay({
  profile,
  phase,
  error,
  logs,
  showLogs,
  onToggleLogs,
  onClose,
}: ConnectingOverlayProps) {
  const title = profile.name || profile.host || "Host";
  const subtitle = `SSH ${profile.host || "?"}:${profile.port || 22}`;

  return (
    <div className="connect-overlay">
      <div className="connect-card">
        <div className="connect-host">
          <div className="connect-os" aria-hidden>
            <svg viewBox="0 0 24 24" width="22" height="22" fill="currentColor">
              <circle cx="12" cy="12" r="10" opacity="0.2" />
              <path d="M7 14.5c1.2 1.6 3 2.5 5 2.5s3.8-.9 5-2.5" fill="none" stroke="currentColor" strokeWidth="1.6" />
              <circle cx="9" cy="10" r="1.1" />
              <circle cx="15" cy="10" r="1.1" />
            </svg>
          </div>
          <div className="connect-host-text">
            <strong>{title}</strong>
            <span>{subtitle}</span>
          </div>
          <button type="button" className="btn ghost connect-logs-btn" onClick={onToggleLogs}>
            {showLogs ? "Hide logs" : "Show logs"}
          </button>
        </div>

        <div className={`connect-progress ${phase}`}>
          <div className="connect-node origin">
            <span className="connect-ring" />
            <span className="connect-icon plug" aria-hidden>
              <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M9 7v4M15 7v4M8 11h8v2a4 4 0 01-4 4h0a4 4 0 01-4-4v-2zM12 17v3" strokeLinecap="round" />
              </svg>
            </span>
          </div>
          <div className="connect-line" />
          <div className="connect-node dest">
            <span className="connect-icon term" aria-hidden>
              <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M4 7l6 5-6 5M12 17h8" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            </span>
          </div>
        </div>

        {phase === "failed" && error && <p className="connect-error">{error}</p>}

        {showLogs && (
          <pre className="connect-log-box">{logs.length ? logs.join("\n") : "暂无日志"}</pre>
        )}

        <button type="button" className="btn connect-close" onClick={onClose}>
          Close
        </button>
      </div>
    </div>
  );
}
