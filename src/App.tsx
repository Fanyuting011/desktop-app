import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import "./App.css";

function App() {
  const [version, setVersion] = useState("…");
  const [status, setStatus] = useState("尚未检查更新");
  void setStatus;

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion("未知"));
  }, []);

  return (
    <main className="page">
      <h1>Desktop Demo</h1>
      <p className="lead">这是 Tauri v2 桌面端基础示例</p>
      <p className="meta">当前版本：{version}</p>
      <button type="button" disabled>
        检查更新（下一任务启用）
      </button>
      <p className="status" role="status">
        {status}
      </p>
    </main>
  );
}

export default App;
