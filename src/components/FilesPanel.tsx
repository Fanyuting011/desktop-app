import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";

interface GatewayProfileLite {
  id: string;
  name: string;
  host: string;
}

interface SessionInfo {
  profileId: string;
  phase: "idle" | "connected" | "proxyOn" | "reconnecting";
}

interface GatewayStatus {
  sessions: SessionInfo[];
}

interface TransferStatus {
  profileId: string | null;
  state: "idle" | "running";
  detail: string | null;
}

interface FilesPanelProps {
  active: boolean;
  profiles: GatewayProfileLite[];
  status: GatewayStatus | null;
}

const POLL_MS = 1000;

function profileName(profile: GatewayProfileLite) {
  return profile.name || profile.host || profile.id;
}

export default function FilesPanel({ active, profiles, status }: FilesPanelProps) {
  const liveIds = useMemo(
    () => new Set(status?.sessions.filter((session) => session.phase !== "idle").map((session) => session.profileId)),
    [status],
  );
  const [profileId, setProfileId] = useState("");
  const [remotePath, setRemotePath] = useState("~");
  const [downloadPath, setDownloadPath] = useState("");
  const [transfer, setTransfer] = useState<TransferStatus>({
    profileId: null,
    state: "idle",
    detail: null,
  });
  const [message, setMessage] = useState("");
  const [dragging, setDragging] = useState(false);
  const dropZoneRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const preferred = profiles.find((profile) => liveIds.has(profile.id)) ?? profiles[0];
    setProfileId((current) => (profiles.some((profile) => profile.id === current) ? current : preferred?.id ?? ""));
  }, [liveIds, profiles]);

  const loadTransferStatus = useCallback(async () => {
    if (!profileId) {
      setTransfer({ profileId: null, state: "idle", detail: null });
      return;
    }
    try {
      setTransfer(await invoke<TransferStatus>("gateway_transfer_status", { profileId }));
    } catch (error) {
      setMessage(String(error));
    }
  }, [profileId]);

  useEffect(() => {
    if (!active) return;
    void loadTransferStatus();
    const id = window.setInterval(() => void loadTransferStatus(), POLL_MS);
    return () => window.clearInterval(id);
  }, [active, loadTransferStatus]);

  const connected = liveIds.has(profileId);
  // Scoped to the currently selected host — a transfer on another host must never disable
  // this host's controls or lock the host picker (see review I3).
  const running = transfer.state === "running" && transfer.profileId === profileId;
  const disabled = !connected || running;

  const upload = useCallback(
    async (paths: string[]) => {
      if (!profileId || disabled || paths.length === 0) return;
      setMessage("");
      try {
        for (const localPath of paths) {
          await invoke("gateway_transfer_upload", {
            profileId,
            localPath,
            remotePath: remotePath.trim() || "~",
          });
        }
        setMessage(`已上传 ${paths.length} 项`);
      } catch (error) {
        setMessage(String(error));
      } finally {
        await loadTransferStatus();
      }
    },
    [disabled, loadTransferStatus, profileId, remotePath],
  );

  // Tauri 2 removes the Tauri-1-only `File.path` extension, so HTML5 `drop` events can never
  // carry a usable filesystem path. Drag-and-drop must instead be driven by the webview-level
  // `onDragDropEvent` API, gated to when the Files panel is active and the pointer is over
  // this panel's drop zone (so switching to Hosts/Logs — or dropping outside the zone — never
  // triggers an upload). See review C2.
  useEffect(() => {
    if (!active) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    const isOverDropZone = (physical: { toLogical: (scale: number) => { x: number; y: number } }) => {
      const zone = dropZoneRef.current;
      if (!zone) return false;
      const scale = window.devicePixelRatio || 1;
      const { x, y } = physical.toLogical(scale);
      const rect = zone.getBoundingClientRect();
      return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
    };

    getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "enter" || payload.type === "over") {
          setDragging(isOverDropZone(payload.position));
        } else if (payload.type === "drop") {
          const over = isOverDropZone(payload.position);
          setDragging(false);
          if (!over) return;
          if (payload.paths.length === 0) {
            setMessage("请使用上传按钮");
            return;
          }
          void upload(payload.paths);
        } else {
          setDragging(false);
        }
      })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch((error) => setMessage(String(error)));

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [active, upload]);

  async function chooseFiles() {
    const selected = await open({ multiple: true });
    if (!selected) return;
    await upload(Array.isArray(selected) ? selected : [selected]);
  }

  async function chooseDirectory() {
    const selected = await open({ directory: true, multiple: false });
    if (!selected || Array.isArray(selected)) return;
    await upload([selected]);
  }

  async function download() {
    if (!profileId || disabled) return;
    const trimmed = downloadPath.trim();
    // Downloading always passes `-r` on the backend, so an unset/`~` path means "recursively
    // pull the entire remote home directory" — a single misclick must not do that silently.
    // See review I4.
    if (trimmed === "" || trimmed === "~") {
      const proceed = window.confirm(
        "未填写远程下载路径，将递归下载整个远程家目录 (~)，可能耗时很长。确定继续吗？",
      );
      if (!proceed) return;
    }
    const localPath = await save({ title: "下载到", defaultPath: "download" });
    if (!localPath) return;
    setMessage("");
    try {
      await invoke("gateway_transfer_download", {
        profileId,
        remotePath: trimmed || "~",
        localPath,
      });
      setMessage("下载完成");
    } catch (error) {
      setMessage(String(error));
    } finally {
      await loadTransferStatus();
    }
  }

  return (
    <section className="files-page">
      <div className="page-head">
        <div>
          <h2>Files</h2>
          <p className="files-note">通过 SCP 在已连接主机和本机之间传输文件。</p>
        </div>
      </div>

      <div className="files-card">
        <label>
          主机
          <select
            className="filter-select"
            value={profileId}
            onChange={(event) => setProfileId(event.target.value)}
            disabled={running}
          >
            {profiles.map((profile) => (
              <option key={profile.id} value={profile.id}>
                {profileName(profile)}{liveIds.has(profile.id) ? "" : "（未连接）"}
              </option>
            ))}
          </select>
        </label>
        <label>
          上传目标路径（远程）
          <input
            value={remotePath}
            onChange={(event) => setRemotePath(event.target.value)}
            placeholder="~"
            disabled={disabled}
          />
        </label>
        <label>
          下载来源路径（远程文件或目录）
          <input
            value={downloadPath}
            onChange={(event) => setDownloadPath(event.target.value)}
            placeholder="例如 ~/logs/app.log（留空将下载整个 ~，需二次确认）"
            disabled={disabled}
          />
        </label>
        <p className="files-overwrite">同名文件将被覆盖</p>

        <div
          ref={dropZoneRef}
          className={dragging ? "files-dropzone dragging" : "files-dropzone"}
        >
          将文件或目录拖到这里上传
        </div>

        <div className="files-actions">
          <button type="button" className="btn primary" disabled={disabled} onClick={() => void chooseFiles()}>
            上传文件
          </button>
          <button type="button" className="btn ghost" disabled={disabled} onClick={() => void chooseDirectory()}>
            上传目录
          </button>
          <button type="button" className="btn ghost" disabled={disabled} onClick={() => void download()}>
            下载远程路径（另存为）
          </button>
        </div>
        {!connected && profileId && <p className="files-hint">请先连接此主机。</p>}
        {running && <p className="files-hint">{transfer.detail ?? "正在传输…"}</p>}
        {message && <p className="banner">{message}</p>}
      </div>
    </section>
  );
}
