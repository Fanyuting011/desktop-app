import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import "./App.css";

async function handleCheckUpdate(
  setStatus: (s: string) => void,
  setBusy: (b: boolean) => void,
) {
  setBusy(true);
  setStatus("正在检查更新…");
  try {
    const update = await check();
    if (!update) {
      setStatus("已是最新版本");
      return;
    }
    setStatus(`发现新版本 ${update.version}，开始下载…`);
    let downloaded = 0;
    let contentLength = 0;
    await update.downloadAndInstall((event) => {
      switch (event.event) {
        case "Started":
          contentLength = event.data.contentLength ?? 0;
          break;
        case "Progress":
          downloaded += event.data.chunkLength;
          if (contentLength > 0) {
            const pct = Math.min(
              100,
              Math.round((downloaded / contentLength) * 100),
            );
            setStatus(`下载中 ${pct}%`);
          } else {
            setStatus(`已下载 ${downloaded} 字节`);
          }
          break;
        case "Finished":
          setStatus("下载完成，准备安装…");
          break;
      }
    });
    setStatus("更新已安装，正在重启…");
    await relaunch();
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    setStatus(`检查更新失败：${message}`);
  } finally {
    setBusy(false);
  }
}

function App() {
  const [version, setVersion] = useState("…");
  const [status, setStatus] = useState("尚未检查更新");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion("未知"));
  }, []);

  return (
    <main className="page">
      <h1>Desktop Demo</h1>
      <p className="lead">这是 Tauri v2 桌面端基础示例</p>
      <p className="meta">当前版本：{version}</p>
      <button
        type="button"
        disabled={busy}
        onClick={() => handleCheckUpdate(setStatus, setBusy)}
      >
        检查更新
      </button>
      <p className="status" role="status">
        {status}
      </p>
    </main>
  );
}

export default App;
