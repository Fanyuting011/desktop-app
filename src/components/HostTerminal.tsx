import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

interface HostTerminalProps {
  profileId: string;
}

// Written once the PTY is ready (first output chunk, or 800ms if the server
// stays quiet) — never touches the user's global shell rc files.
const INJECT_CMD = "source ~/.outgate/path.sh && outgate on\n";
const INJECT_TIMEOUT_MS = 800;

export default function HostTerminal({ profileId }: HostTerminalProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const term = new Terminal({
      // convertEol breaks readline Up-arrow redraw (cursor column desync).
      convertEol: false,
      fontFamily: '"SF Mono", "Menlo", "Cascadia Mono", ui-monospace, monospace',
      fontSize: 13,
      lineHeight: 1.3,
      cursorBlink: true,
      cursorStyle: "block",
      scrollback: 5000,
      theme: {
        background: "#0f1720",
        foreground: "#d7e2ec",
        cursor: "#d7e2ec",
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(container);

    let cancelled = false;
    let lastSent = { cols: 0, rows: 0 };
    let resizeSynced = false;

    const sendResize = async (force = false) => {
      if (cancelled) return false;
      try {
        fit.fit();
      } catch {
        return false;
      }
      const cols = term.cols;
      const rows = term.rows;
      if (cols < 2 || rows < 1) return false;
      if (!force && cols === lastSent.cols && rows === lastSent.rows && resizeSynced) {
        return true;
      }
      try {
        await invoke("gateway_terminal_resize", { profileId, cols, rows });
        lastSent = { cols, rows };
        resizeSynced = true;
        return true;
      } catch {
        // PTY often not open yet (HostTerminal mounts before gateway_terminal_open finishes).
        resizeSynced = false;
        return false;
      }
    };

    // Keep trying until the backend PTY exists and accepts the real size.
    // Without this, bash stays at 80x24 while xterm is wider → Up-arrow
    // redraw walks into previous output and overwrites it.
    const resizeRetry = window.setInterval(() => {
      void sendResize();
    }, 250);
    window.setTimeout(() => window.clearInterval(resizeRetry), 20_000);

    requestAnimationFrame(() => {
      void sendResize(true);
    });

    const injectedRef = { current: false };
    const injectOnce = () => {
      if (injectedRef.current) return;
      injectedRef.current = true;
      void (async () => {
        await sendResize(true);
        try {
          await invoke("gateway_terminal_write", { profileId, data: INJECT_CMD });
        } catch (error) {
          console.warn(`Failed to inject OutGate environment for profile ${profileId}`, error);
        }
      })();
    };
    let injectTimer = window.setTimeout(injectOnce, INJECT_TIMEOUT_MS);
    const unlisteners: Array<() => void> = [];
    const track = (fn: () => void) => {
      if (cancelled) fn();
      else unlisteners.push(fn);
    };

    listen<string>(`terminal-output-${profileId}`, (event) => {
      term.write(event.payload);
      void sendResize();
      injectOnce();
    }).then(track);

    const dataDisposable = term.onData((data) => {
      invoke("gateway_terminal_write", { profileId, data }).catch(() => {});
    });

    const resizeObserver = new ResizeObserver(() => {
      void sendResize(true);
    });
    resizeObserver.observe(container);

    listen(`terminal-reconnect-${profileId}`, () => {
      term.reset();
      term.writeln("\x1b[2m— 隧道已重连，终端会话已重建 —\x1b[0m");
      injectedRef.current = false;
      resizeSynced = false;
      lastSent = { cols: 0, rows: 0 };
      window.clearTimeout(injectTimer);
      injectTimer = window.setTimeout(injectOnce, INJECT_TIMEOUT_MS);
      void sendResize(true);
    }).then(track);

    return () => {
      cancelled = true;
      window.clearTimeout(injectTimer);
      window.clearInterval(resizeRetry);
      resizeObserver.disconnect();
      dataDisposable.dispose();
      unlisteners.forEach((fn) => fn());
      term.dispose();
    };
  }, [profileId]);

  return <div ref={containerRef} className="host-terminal" />;
}
