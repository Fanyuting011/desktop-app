import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

interface HostTerminalProps {
  profileId: string;
}

// Written once the PTY is ready — never touches the user's global shell rc files.
// Use CR (`\r`) like a real Enter key: Windows ConPTY / OpenSSH often ignore bare LF
// and never submit the line, so the inject appears to "not run" on Windows.
const INJECT_CMD = ". ~/.outgate/path.sh && outgate on\r";
const INJECT_TIMEOUT_MS = 1200;
const INJECT_RETRY_MS = 500;
const INJECT_MAX_ATTEMPTS = 8;

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
    const injectingRef = { current: false };
    let injectAttempts = 0;
    let injectRetryTimer = 0;

    const injectOnce = (reason: "output" | "timeout" | "retry") => {
      if (cancelled || injectedRef.current || injectingRef.current) return;
      injectingRef.current = true;
      void (async () => {
        injectAttempts += 1;
        await sendResize(true);
        try {
          await invoke("gateway_terminal_write", { profileId, data: INJECT_CMD });
          injectedRef.current = true;
        } catch (error) {
          console.warn(
            `Failed to inject OutGate environment for profile ${profileId} (${reason}, attempt ${injectAttempts})`,
            error,
          );
          if (!cancelled && injectAttempts < INJECT_MAX_ATTEMPTS) {
            injectRetryTimer = window.setTimeout(() => injectOnce("retry"), INJECT_RETRY_MS);
          }
        } finally {
          injectingRef.current = false;
        }
      })();
    };

    let injectTimer = window.setTimeout(() => injectOnce("timeout"), INJECT_TIMEOUT_MS);
    const unlisteners: Array<() => void> = [];
    const track = (fn: () => void) => {
      if (cancelled) fn();
      else unlisteners.push(fn);
    };

    listen<string>(`terminal-output-${profileId}`, (event) => {
      term.write(event.payload);
      void sendResize();
      injectOnce("output");
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
      injectAttempts = 0;
      resizeSynced = false;
      lastSent = { cols: 0, rows: 0 };
      window.clearTimeout(injectTimer);
      window.clearTimeout(injectRetryTimer);
      injectTimer = window.setTimeout(() => injectOnce("timeout"), INJECT_TIMEOUT_MS);
      void sendResize(true);
    }).then(track);

    return () => {
      cancelled = true;
      window.clearTimeout(injectTimer);
      window.clearTimeout(injectRetryTimer);
      window.clearInterval(resizeRetry);
      resizeObserver.disconnect();
      dataDisposable.dispose();
      unlisteners.forEach((fn) => fn());
      term.dispose();
    };
  }, [profileId]);

  return <div ref={containerRef} className="host-terminal" />;
}
