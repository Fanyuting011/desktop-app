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
      convertEol: true,
      fontFamily: '"SF Mono", "Menlo", ui-monospace, monospace',
      fontSize: 12,
      lineHeight: 1.35,
      cursorBlink: true,
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
    try {
      fit.fit();
    } catch {
      /* container not laid out yet */
    }

    const injectedRef = { current: false };
    const injectOnce = () => {
      if (injectedRef.current) return;
      injectedRef.current = true;
      invoke("gateway_terminal_write", { profileId, data: INJECT_CMD }).catch(() => {});
    };
    const injectTimer = window.setTimeout(injectOnce, INJECT_TIMEOUT_MS);

    let cancelled = false;
    let unlisten: (() => void) | undefined;
    listen<string>(`terminal-output-${profileId}`, (event) => {
      term.write(event.payload);
      injectOnce();
    }).then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlisten = fn;
      }
    });

    const dataDisposable = term.onData((data) => {
      invoke("gateway_terminal_write", { profileId, data }).catch(() => {});
    });

    const sendResize = () => {
      try {
        fit.fit();
      } catch {
        return;
      }
      invoke("gateway_terminal_resize", {
        profileId,
        cols: term.cols,
        rows: term.rows,
      }).catch(() => {});
    };
    const resizeObserver = new ResizeObserver(sendResize);
    resizeObserver.observe(container);
    sendResize();

    return () => {
      cancelled = true;
      window.clearTimeout(injectTimer);
      resizeObserver.disconnect();
      dataDisposable.dispose();
      unlisten?.();
      term.dispose();
    };
  }, [profileId]);

  return <div ref={containerRef} className="host-terminal" />;
}
