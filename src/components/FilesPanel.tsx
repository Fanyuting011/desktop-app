import { useCallback, useEffect, useMemo, useState, type DragEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";

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

interface TauriFile extends File {
  path?: string;
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
  const [transfer, setTransfer] = useState<TransferStatus>({
    profileId: null,
    state: "idle",
    detail: null,
  });
  const [message, setMessage] = useState("");
  const [dragging, setDragging] = useState(false);

  useEffect(() => {
    const preferred = profiles.find((profile) => liveIds.has(profile.id)) ?? profiles[0];
    setProfileId((current) => (profiles.some((profile) => profile.id === current) ? current : preferred?.id ?? ""));
  }, [liveIds, profiles]);

  const loadTransferStatus = useCallback(async () => {
    try {
      setTransfer(await invoke<TransferStatus>("gateway_transfer_status"));
    } catch (error) {
      setMessage(String(error));
    }
  }, []);

  useEffect(() => {
    if (!active) return;
    void loadTransferStatus();
    const id = window.setInterval(() => void loadTransferStatus(), POLL_MS);
    return () => window.clearInterval(id);
  }, [active, loadTransferStatus]);

  const connected = liveIds.has(profileId);
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
    const localPath = await save({ title: "下载到", defaultPath: "download" });
    if (!localPath) return;
    setMessage("");
    try {
      await invoke("gateway_transfer_download", {
        profileId,
        remotePath: remotePath.trim() || "~",
        localPath,
      });
      setMessage("下载完成");
    } catch (error) {
      setMessage(String(error));
    } finally {
      await loadTransferStatus();
    }
  }

  function onDrop(event: DragEvent<HTMLDivElement>) {
    event.preventDefault();
    setDragging(false);
    const paths = Array.from(event.dataTransfer.files)
      .map((file) => (file as TauriFile).path)
      .filter((path): path is string => Boolean(path));
    if (paths.length === 0) {
      setMessage("请使用上传按钮");
      return;
    }
    void upload(paths);
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
            disabled={transfer.state === "running"}
          >
            {profiles.map((profile) => (
              <option key={profile.id} value={profile.id}>
                {profileName(profile)}{liveIds.has(profile.id) ? "" : "（未连接）"}
              </option>
            ))}
          </select>
        </label>
        <label>
          远程路径
          <input
            value={remotePath}
            onChange={(event) => setRemotePath(event.target.value)}
            placeholder="~"
            disabled={disabled}
          />
        </label>
        <p className="files-overwrite">同名文件将被覆盖</p>

        <div
          className={dragging ? "files-dropzone dragging" : "files-dropzone"}
          onDragOver={(event) => {
            event.preventDefault();
            if (!disabled) setDragging(true);
          }}
          onDragLeave={() => setDragging(false)}
          onDrop={onDrop}
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
            下载（另存为）
          </button>
        </div>
        {!connected && profileId && <p className="files-hint">请先连接此主机。</p>}
        {running && <p className="files-hint">{transfer.detail ?? "正在传输…"}</p>}
        {message && <p className="banner">{message}</p>}
      </div>
    </section>
  );
}
