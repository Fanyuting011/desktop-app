#!/usr/bin/env python3
"""Generate Tauri updater latest.json for a GitHub Release asset."""

from __future__ import annotations

import argparse
import json
from datetime import datetime, timezone
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("--github-repo", required=True, help="OWNER/REPO")
    parser.add_argument("--platform", required=True, help="e.g. darwin-aarch64")
    parser.add_argument("--asset", required=True, help="uploaded filename")
    parser.add_argument("--signature-file", required=True)
    parser.add_argument("--notes", default="")
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    signature = Path(args.signature_file).read_text(encoding="utf-8").strip()
    if not signature:
        raise SystemExit(f"empty signature file: {args.signature_file}")

    tag = f"v{args.version.lstrip('v')}"
    version = args.version.lstrip("v")
    url = (
        f"https://github.com/{args.github_repo}/releases/download/"
        f"{tag}/{args.asset}"
    )

    payload = {
        "version": version,
        "notes": args.notes or f"Desktop Demo {version}",
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": {
            args.platform: {
                "signature": signature,
                "url": url,
            }
        },
    }

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(
        json.dumps(payload, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
