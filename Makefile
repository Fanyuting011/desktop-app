# Desktop Demo release helpers
#
# Usage:
#   export TAURI_SIGNING_PRIVATE_KEY_PASSWORD='...'   # optional
#   make publish          # build + latest.json + upload GitHub Release
#   make package          # build + latest.json only (no upload)
#
# Requires:
#   - signing key: ~/.tauri/desktop-demo.key  or  $TAURI_SIGNING_PRIVATE_KEY
#   - GitHub CLI:  gh auth login

SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c

GITHUB_REPO ?= okboy32/desktop_app
KEY_FILE ?= $(HOME)/.tauri/desktop-demo.key
BUNDLE_DIR := src-tauri/target/release/bundle
RELEASE_DIR := release
SKIP_UPLOAD ?= 0
PUSH_GIT ?= 1

VERSION := $(shell node -p "require('./package.json').version")
TAG := v$(VERSION)

UNAME_M := $(shell uname -m)
ifeq ($(UNAME_M),arm64)
  DARWIN_TARGET := darwin-aarch64
else ifeq ($(UNAME_M),aarch64)
  DARWIN_TARGET := darwin-aarch64
else
  DARWIN_TARGET := darwin-x86_64
endif

.PHONY: help build package upload publish clean-release ensure-gh push-git

help:
	@echo "Targets:"
	@echo "  make build     - npm run tauri build (needs signing key)"
	@echo "  make package   - build + stage $(RELEASE_DIR)/ + write latest.json"
	@echo "  make upload    - upload $(RELEASE_DIR)/* to GitHub Release $(TAG)"
	@echo "  make publish   - package + push git tag (optional) + upload release"
	@echo ""
	@echo "Variables:"
	@echo "  SKIP_UPLOAD=1  - make publish stops after package"
	@echo "  PUSH_GIT=0     - skip git commit check / tag push"
	@echo ""
	@echo "Current version: $(VERSION)  tag: $(TAG)  platform: $(DARWIN_TARGET)"

build:
	@if [ -z "$${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then \
	  if [ -f "$(KEY_FILE)" ]; then \
	    export TAURI_SIGNING_PRIVATE_KEY="$$(cat "$(KEY_FILE)")"; \
	  else \
	    echo "error: set TAURI_SIGNING_PRIVATE_KEY or create $(KEY_FILE)"; \
	    exit 1; \
	  fi; \
	fi; \
	export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"; \
	echo "==> Building Desktop Demo $(VERSION)…"; \
	npm run tauri build

clean-release:
	rm -rf "$(RELEASE_DIR)"

package: build
	@mkdir -p "$(RELEASE_DIR)"
	@echo "==> Collecting artifacts into $(RELEASE_DIR)/…"
	@tar_gz=$$(ls "$(BUNDLE_DIR)/macos/"*.app.tar.gz 2>/dev/null | head -1); \
	sig=$${tar_gz}.sig; \
	dmg=$$(ls "$(BUNDLE_DIR)/dmg/"*.dmg 2>/dev/null | head -1); \
	if [ -z "$$tar_gz" ] || [ ! -f "$$sig" ]; then \
	  echo "error: missing updater artifacts under $(BUNDLE_DIR)/macos/"; \
	  echo "hint: ensure createUpdaterArtifacts is true and signing key is set"; \
	  exit 1; \
	fi; \
	safe_tar="Desktop.Demo.app.tar.gz"; \
	safe_sig="Desktop.Demo.app.tar.gz.sig"; \
	cp "$$tar_gz" "$(RELEASE_DIR)/$$safe_tar"; \
	cp "$$sig" "$(RELEASE_DIR)/$$safe_sig"; \
	if [ -n "$$dmg" ]; then \
	  safe_dmg=$$(basename "$$dmg" | tr ' ' '.'); \
	  cp "$$dmg" "$(RELEASE_DIR)/$$safe_dmg"; \
	fi; \
	python3 scripts/gen-latest-json.py \
	  --version "$(VERSION)" \
	  --github-repo "$(GITHUB_REPO)" \
	  --platform "$(DARWIN_TARGET)" \
	  --asset "$$safe_tar" \
	  --signature-file "$(RELEASE_DIR)/$$safe_sig" \
	  --notes "Desktop Demo $(VERSION)" \
	  --out "$(RELEASE_DIR)/latest.json"; \
	cp "$(RELEASE_DIR)/latest.json" latest.json; \
	echo "==> Packaged:"; \
	ls -lh "$(RELEASE_DIR)"

ensure-gh:
	@command -v gh >/dev/null || { echo "error: install GitHub CLI (gh)"; exit 1; }
	@gh auth status >/dev/null 2>&1 || { echo "error: run: gh auth login"; exit 1; }

push-git:
	@if [ "$(PUSH_GIT)" != "1" ]; then \
	  echo "==> SKIP push-git (PUSH_GIT=$(PUSH_GIT))"; \
	  exit 0; \
	fi
	@if ! git diff --quiet || ! git diff --cached --quiet; then \
	  echo "error: working tree dirty — commit version bumps first, then make publish"; \
	  git status --short; \
	  exit 1; \
	fi
	@echo "==> Ensuring tag $(TAG) exists and is pushed…"
	@if git rev-parse "$(TAG)" >/dev/null 2>&1; then \
	  echo "local tag $(TAG) already exists"; \
	else \
	  git tag "$(TAG)"; \
	fi
	@git push origin HEAD
	@git push origin "$(TAG)"

upload: ensure-gh
	@if [ ! -f "$(RELEASE_DIR)/latest.json" ]; then \
	  echo "error: $(RELEASE_DIR)/latest.json missing — run make package first"; \
	  exit 1; \
	fi
	@echo "==> Uploading $(RELEASE_DIR)/* to GitHub Release $(TAG)…"
	@if gh release view "$(TAG)" --repo "$(GITHUB_REPO)" >/dev/null 2>&1; then \
	  gh release upload "$(TAG)" "$(RELEASE_DIR)"/* \
	    --repo "$(GITHUB_REPO)" \
	    --clobber; \
	  echo "==> Updated existing release $(TAG)"; \
	else \
	  gh release create "$(TAG)" "$(RELEASE_DIR)"/* \
	    --repo "$(GITHUB_REPO)" \
	    --title "Desktop Demo $(TAG)" \
	    --notes "$$(printf '%s\n\n%s\n%s\n' \
	      "Desktop Demo $(VERSION)" \
	      "- macOS 安装包与自动更新产物" \
	      "- 请用已安装的旧版本点击「检查更新」验证")" \
	    --latest; \
	  echo "==> Created release $(TAG)"; \
	fi
	@echo ""
	@echo "Release: https://github.com/$(GITHUB_REPO)/releases/tag/$(TAG)"
	@echo "latest.json: https://github.com/$(GITHUB_REPO)/releases/latest/download/latest.json"

publish: package
	@if [ "$(SKIP_UPLOAD)" = "1" ]; then \
	  echo "==> SKIP upload (SKIP_UPLOAD=1). Files are in $(RELEASE_DIR)/"; \
	  exit 0; \
	fi
	@$(MAKE) push-git
	@$(MAKE) upload
