.PHONY: build build-ingress build-all build-tunnel tunnel run run-release run-ingress run-ingress-release \
	test check package package-clean package-amd64 package-arm64 package-deb package-rpm \
	package-proxy package-ingress package-tunnel package-tunnel-amd64 package-tunnel-arm64 package-helm helm-package \
	release release-amd release-arm release-helm publish-helm \
	deploy deploy-package deploy-package-ingress deploy-remote \
	deploy-deb deploy-deb-ingress deploy-deb-arm deploy-rpm deploy-rpm-ingress deploy-rpm-arm deploy-rpm-arm64 \
	deploy-rpm-tunnel deploy-rpm-tunnel-server deploy-rpm-tunnel-client apply-ingress-rbac \
	deploy-ingress-helm deploy-ingress deploy-cloud deploy-285h uninstall-legacy-ingress-helm \
	docker-ingress docker-ingress-push docker-ingress-multi \
	install-admin admin-dist fix-perms dev dev-vite dev-serve dev-admin dev-stop

CARGO ?= cargo
INGRESS_FEATURES ?= ingress
PERTISK_DB_PATH ?= ./data/proxy.sqlite
# Optional one-time migration when DB is empty (legacy routes.yaml)
ROUTES_CONFIG ?=
PROXY_CARGO_FEATURES = --features admin
PROXY_MODE ?= performance
LOG_LEVEL ?= info
ENABLE_H3 ?= true

# Dev listen addresses (DNS-ready: 80 + 443/tcp + 443/udp). On macOS use: sudo make dev
DEV_LISTEN_HTTP ?= 0.0.0.0:80
DEV_LISTEN_HTTPS ?= 0.0.0.0:443
DEV_LISTEN_H3_UDP ?= 0.0.0.0:443
DEV_MANAGEMENT_ADDR ?= 0.0.0.0:9080

# When using `sudo make dev`, run pnpm as the invoking user so node_modules stays writable.
DEV_USER ?= $(if $(SUDO_USER),$(SUDO_USER),$(USER))
RUN_AS_USER = $(if $(filter root,$(USER)),sudo -u $(DEV_USER) ,)

VERSION ?= $(shell git describe --tags --always 2>/dev/null | sed 's/^v//' || echo "0.1.0")
PACKAGE_TARGET ?= all
BUILDER_NAME ?= pertisk-proxy-package
CACHE_DIR ?= .buildx-cache/release

# Remote deploy — use DEPLOY_HOST=user@host or REMOTE_USER + REMOTE_HOST
DEPLOY_HOST ?=
DEPLOY_ARCH ?= auto
# Map DEPLOY_ARCH (amd64|arm64) to package arch names
RPM_ARCH = $(if $(filter arm64,$(DEPLOY_ARCH)),aarch64,x86_64)
DEB_ARCH = $(DEPLOY_ARCH)
DEPLOY_BIN ?= pertisk-proxy
DEPLOY_PKG ?= auto
DEPLOY_SSH_OPTS ?=

# Remote build + deploy (./build/deploy-deb.sh / deploy-rpm.sh)
REMOTE_HOST ?=
REMOTE_USER ?= root
PACKAGE_NAME ?= pertisk-proxy
REMOTE_PATH ?= /tmp
PACKAGE_CLEAN ?= 1
PACKAGE_BUILD ?= 1

build:
	$(CARGO) build --release --bin pertisk-proxy --features admin

build-ingress:
	$(CARGO) build --release --bin pertisk-proxy-ingress --features $(INGRESS_FEATURES)

build-all: build build-ingress

# Reverse tunnel binaries (local client ↔ VPS server)
tunnel build-tunnel:
	$(CARGO) build --release -p pertisk-tunnel-server -p pertisk-tunnel-client

# Tunnel DEB/RPM (both server + client by default)
#   make package-tunnel VERSION=0.1.80
#   make package-tunnel-amd64 VERSION=0.1.80
#   make deploy-rpm-tunnel DEPLOY_HOST=user@vps VERSION=0.1.80
#   make deploy-rpm-tunnel-server DEPLOY_HOST=user@vps
#   make deploy-rpm-tunnel-client DEPLOY_HOST=user@laptop
package-tunnel: package-tunnel-amd64

package-tunnel-amd64:
	chmod +x build/package-tunnel.sh build/deploy-rpm-tunnel.sh
	./build/package-tunnel.sh amd64 $(VERSION) both

package-tunnel-arm64:
	chmod +x build/package-tunnel.sh build/deploy-rpm-tunnel.sh
	./build/package-tunnel.sh arm64 $(VERSION) both

deploy-rpm-tunnel:
	chmod +x build/deploy-rpm-tunnel.sh
	DEPLOY_HOST="$(DEPLOY_HOST)" REMOTE_HOST="$(REMOTE_HOST)" REMOTE_USER="$(REMOTE_USER)" \
		VERSION="$(VERSION)" PACKAGE_BUILD="$(PACKAGE_BUILD)" PACKAGE_CLEAN="$(PACKAGE_CLEAN)" \
		DEPLOY_ARCH="$(DEPLOY_ARCH)" DEPLOY_SSH_OPTS="$(DEPLOY_SSH_OPTS)" \
		TUNNEL_PKG="$(or $(TUNNEL_PKG),both)" \
		./build/deploy-rpm-tunnel.sh

deploy-rpm-tunnel-server:
	$(MAKE) deploy-rpm-tunnel TUNNEL_PKG=server DEPLOY_HOST="$(DEPLOY_HOST)" VERSION="$(VERSION)"

deploy-rpm-tunnel-client:
	$(MAKE) deploy-rpm-tunnel TUNNEL_PKG=client DEPLOY_HOST="$(DEPLOY_HOST)" VERSION="$(VERSION)"

check:
	$(CARGO) check --features $(INGRESS_FEATURES)
	$(CARGO) check -p pertisk-tunnel-proto -p pertisk-tunnel-server -p pertisk-tunnel-client

# Admin UI (React + Vite)
install-admin:
	cd admin && $(RUN_AS_USER)pnpm install

# Fix root-owned files after `sudo make dev` (requires your password once).
fix-perms:
	@if [ "$$(id -u)" -ne 0 ]; then \
		echo "Run: sudo make fix-perms"; exit 1; \
	fi
	chown -R $(DEV_USER):staff admin/node_modules admin/dist data target 2>/dev/null || true
	@echo "Fixed ownership for admin/node_modules, admin/dist, data/, and target/"

admin-dist:
	@if [ -d admin/dist/assets ] && [ ! -w admin/dist/assets ]; then \
		echo "admin/dist is not writable (usually from 'sudo make dev'). Run: sudo make fix-perms"; exit 1; \
	fi
	@echo "Checking admin UI build cache..."
	@if [ -d admin/dist ] && [ -z "$$(find admin/src admin/public admin/index.html admin/package.json admin/pnpm-lock.yaml -type f -newer admin/dist 2>/dev/null | head -n 1)" ]; then \
		echo "admin/dist is up to date; skipping build."; \
	else \
		echo "Building admin UI..."; \
		if [ ! -d admin/node_modules ]; then $(MAKE) install-admin; fi; \
		rm -rf admin/dist 2>/dev/null || { \
			echo "Cannot clean admin/dist (files may be owned by root after 'sudo make dev')."; \
			echo "Run: sudo make fix-perms"; exit 1; \
		}; \
		cd admin && $(RUN_AS_USER)pnpm run build; \
	fi

dev-admin:
	cd admin && $(RUN_AS_USER)pnpm dev

# Stop stale dev processes (cargo-watch, pertisk-proxy, vite)
dev-stop:
	-pkill -f 'cargo-watch.*pertisk-proxy' 2>/dev/null
	-pkill -f 'cargo-watch watch' 2>/dev/null
	@sleep 0.3
	-pkill -f 'target/debug/pertisk-proxy' 2>/dev/null
	-pkill -f 'target/release/pertisk-proxy' 2>/dev/null
	@if command -v lsof >/dev/null 2>&1; then \
		for port in 80 443 8080 8443 9080 5173; do \
			pids=$$(lsof -tiTCP:$$port -sTCP:LISTEN 2>/dev/null); \
			[ -n "$$pids" ] && kill $$pids 2>/dev/null || true; \
		done; \
		pids=$$(lsof -tiUDP:443 2>/dev/null); \
		[ -n "$$pids" ] && kill $$pids 2>/dev/null || true; \
		sleep 0.3; \
		for port in 80 443 8080 8443 9080 5173; do \
			pids=$$(lsof -tiTCP:$$port -sTCP:LISTEN 2>/dev/null); \
			[ -n "$$pids" ] && kill -9 $$pids 2>/dev/null || true; \
		done; \
		pids=$$(lsof -tiUDP:443 2>/dev/null); \
		[ -n "$$pids" ] && kill -9 $$pids 2>/dev/null || true; \
	fi
	@echo "Stopped dev processes (if any were running)."

DEV_PREFIX = build/dev-prefix-log.sh

# Backend + admin UI (management API :9080). Serves built admin/dist; UI rebuilds on change.
# macOS requires root for 80/443: sudo make dev
dev: admin-dist dev-stop
	chmod +x $(DEV_PREFIX)
	PERTISK_DB_PATH=$(PERTISK_DB_PATH) $(if $(ROUTES_CONFIG),ROUTES_CONFIG=$(ROUTES_CONFIG),) ENABLE_H3=$(ENABLE_H3) PERTISK_PROXY_MODE=$(PROXY_MODE) \
		PERTISK_LOG_LEVEL=$(LOG_LEVEL) \
		LISTEN_HTTP=$(DEV_LISTEN_HTTP) LISTEN_HTTPS=$(DEV_LISTEN_HTTPS) LISTEN_H3_UDP=$(DEV_LISTEN_H3_UDP) \
		PERTISK_MANAGEMENT_ADDR=$(DEV_MANAGEMENT_ADDR) \
		$(CARGO) watch -i admin -x 'run --bin pertisk-proxy $(PROXY_CARGO_FEATURES)' 2>&1 | $(DEV_PREFIX) proxy & \
	(cd admin && $(RUN_AS_USER)pnpm run build:watch 2>&1 | $(DEV_PREFIX) admin) & \
	wait

# Vite hot-reload on http://127.0.0.1:5173 only (local; do not proxy Vite — WebSocket breaks through Pingora).
dev-vite: dev-stop
	chmod +x $(DEV_PREFIX)
	PERTISK_DB_PATH=$(PERTISK_DB_PATH) $(if $(ROUTES_CONFIG),ROUTES_CONFIG=$(ROUTES_CONFIG),) ENABLE_H3=$(ENABLE_H3) PERTISK_PROXY_MODE=$(PROXY_MODE) \
		PERTISK_LOG_LEVEL=$(LOG_LEVEL) \
		LISTEN_HTTP=$(DEV_LISTEN_HTTP) LISTEN_HTTPS=$(DEV_LISTEN_HTTPS) LISTEN_H3_UDP=$(DEV_LISTEN_H3_UDP) \
		PERTISK_MANAGEMENT_ADDR=$(DEV_MANAGEMENT_ADDR) \
		$(CARGO) watch -i admin -x 'run --bin pertisk-proxy $(PROXY_CARGO_FEATURES)' 2>&1 | $(DEV_PREFIX) proxy & \
	(cd admin && API_PROXY_TARGET=http://$(DEV_MANAGEMENT_ADDR) $(RUN_AS_USER)pnpm dev 2>&1 | $(DEV_PREFIX) vite) & \
	wait

# Alias for `make dev`
dev-serve: dev

# Proxy mode — standalone reverse proxy
run:
	PERTISK_DB_PATH=$(PERTISK_DB_PATH) PERTISK_ADMIN_PASSWORD=admin $(if $(ROUTES_CONFIG),ROUTES_CONFIG=$(ROUTES_CONFIG),) ENABLE_H3=$(ENABLE_H3) PERTISK_PROXY_MODE=$(PROXY_MODE) PERTISK_LOG_LEVEL=$(LOG_LEVEL) $(CARGO) run --bin pertisk-proxy --features admin

run-release: admin-dist
	PERTISK_DB_PATH=$(PERTISK_DB_PATH) $(if $(ROUTES_CONFIG),ROUTES_CONFIG=$(ROUTES_CONFIG),) ENABLE_H3=$(ENABLE_H3) PERTISK_PROXY_MODE=$(PROXY_MODE) PERTISK_LOG_LEVEL=$(LOG_LEVEL) $(CARGO) run --release --bin pertisk-proxy --features admin

# Ingress mode — Kubernetes Ingress controller (uses current kubeconfig)
run-ingress:
	ENABLE_H3=$(ENABLE_H3) PERTISK_LOG_LEVEL=$(LOG_LEVEL) $(CARGO) run --bin pertisk-proxy-ingress --features $(INGRESS_FEATURES)

run-ingress-release:
	ENABLE_H3=$(ENABLE_H3) PERTISK_LOG_LEVEL=$(LOG_LEVEL) $(CARGO) run --release --bin pertisk-proxy-ingress --features $(INGRESS_FEATURES)

test:
	$(CARGO) test --features $(INGRESS_FEATURES)

# Run library unit tests with 95% line coverage (see tarpaulin.toml).
test-coverage:
	cargo tarpaulin --config tarpaulin.toml --lib

# --- Packaging: DEB + RPM + tarball (Docker cross-build on macOS) → release/ ---
# Requires: docker (buildx cross-compile + fpm only; no runtime images).
# make package              — amd64 + arm64, both binaries
# make package-amd64        — amd64 only
# make package-proxy        — proxy binary only
# make package-ingress      — ingress binary only (Kubernetes controller)

package-clean:
	rm -f pertisk-proxy-linux-amd64 pertisk-proxy-linux-arm64 \
		pertisk-proxy-ingress-linux-amd64 pertisk-proxy-ingress-linux-arm64 \
		pertisk-tunnel-server-linux-amd64 pertisk-tunnel-server-linux-arm64 \
		pertisk-tunnel-client-linux-amd64 pertisk-tunnel-client-linux-arm64
	@echo "Removed Linux binaries; next package build will rebuild via Docker."

package-amd64: admin-dist
	chmod +x build/package.sh build/deb-rpm.sh build/deploy-remote.sh build/deploy-deb.sh build/deploy-rpm.sh
	./build/package.sh amd64 $(VERSION) $(PACKAGE_TARGET)

package-arm64: admin-dist
	chmod +x build/package.sh build/deb-rpm.sh build/deploy-remote.sh build/deploy-deb.sh build/deploy-rpm.sh
	./build/package.sh arm64 $(VERSION) $(PACKAGE_TARGET)

package: package-amd64 package-arm64
	@echo "Done. See release/"

package-proxy:
	$(MAKE) package-amd64 PACKAGE_TARGET=proxy
	$(MAKE) package-arm64 PACKAGE_TARGET=proxy

package-ingress:
	$(MAKE) package-amd64 PACKAGE_TARGET=ingress
	$(MAKE) package-arm64 PACKAGE_TARGET=ingress

package-deb: package
package-rpm: package

# Release = DEB/RPM packages only (no container images)
release:
	$(MAKE) package-clean
	$(MAKE) package VERSION=$(VERSION)
	$(MAKE) package-tunnel VERSION=$(VERSION)

release-amd:
	$(MAKE) package-clean
	$(MAKE) package-amd64 VERSION=$(VERSION)
	$(MAKE) package-tunnel-amd64 VERSION=$(VERSION)

release-arm:
	$(MAKE) package-clean
	$(MAKE) package-arm64 VERSION=$(VERSION)
	$(MAKE) package-tunnel-arm64 VERSION=$(VERSION)

release-proxy:
	$(MAKE) package-clean
	$(MAKE) package-proxy VERSION=$(VERSION)

release-ingress:
	$(MAKE) package-clean
	$(MAKE) package-ingress VERSION=$(VERSION)

# --- Helm chart: package + publish (pertisk-ingress) ---
# make package-helm VERSION=0.1.74   — helm package → release/pertisk-ingress-*.tgz
# make release-helm VERSION=0.1.74   — package + upload to chart repo
# Auth for release-helm: HELM_CHART_TOKEN=...  or  HELM_USER=... HELM_PASSWORD=...
HELM_CHART_REPO_URL ?= https://chart.tools.pertisk.com
HELM_CHART_DIR ?= deploy/helm/pertisk-ingress
HELM_CHART_TOKEN ?=
HELM_USER ?=
HELM_PASSWORD ?=

package-helm helm-package:
	chmod +x build/publish-helm-ingress.sh
	VERSION="$(VERSION)" PACKAGE_ONLY=1 \
		HELM_CHART_REPO_URL="$(HELM_CHART_REPO_URL)" HELM_CHART_DIR="$(HELM_CHART_DIR)" \
		./build/publish-helm-ingress.sh

release-helm publish-helm:
	chmod +x build/publish-helm-ingress.sh
	VERSION="$(VERSION)" PACKAGE_ONLY=0 \
		HELM_CHART_REPO_URL="$(HELM_CHART_REPO_URL)" HELM_CHART_DIR="$(HELM_CHART_DIR)" \
		HELM_CHART_TOKEN="$(HELM_CHART_TOKEN)" HELM_USER="$(HELM_USER)" HELM_PASSWORD="$(HELM_PASSWORD)" \
		./build/publish-helm-ingress.sh

# One-time: IngressClass + ClusterRole for external systemd ingress controller
apply-ingress-rbac:
	kubectl apply -f deploy/kubernetes-rbac.yaml

# --- Docker: ingress controller image (buildx) ---
# make docker-ingress              — build local single-arch image (--load, native platform)
# make docker-ingress-push         — build + push multi-arch manifest (linux/amd64 + linux/arm64)
# make docker-ingress-multi        — alias for docker-ingress-push
# make deploy-ingress              — docker-ingress-push + helm upgrade (full pipeline)
# Kubelet/containerd auto-selects the node arch when pulling a multi-arch tag (no nodeSelector).
INGRESS_BUILD_PLATFORMS ?= linux/amd64,linux/arm64
CACHE_BACKEND ?= registry
HARBOR_INGRESS_IMAGE ?= harbor.tools.pertisk.com/pertisk-proxy/ingress
INGRESS_DOCKERFILE ?= docker/Dockerfile.ingress

docker-ingress: admin-dist
	chmod +x build/ingress-image.sh
	VERSION="$(VERSION)" HARBOR_INGRESS_IMAGE="$(HARBOR_INGRESS_IMAGE)" \
		INGRESS_DOCKERFILE="$(INGRESS_DOCKERFILE)" ./build/ingress-image.sh "$(VERSION)"

docker-ingress-push docker-ingress-multi: admin-dist
	chmod +x build/ingress-image.sh
	VERSION="$(VERSION)" HARBOR_INGRESS_IMAGE="$(HARBOR_INGRESS_IMAGE)" \
		INGRESS_DOCKERFILE="$(INGRESS_DOCKERFILE)" \
		PLATFORMS="$(INGRESS_BUILD_PLATFORMS)" CACHE_BACKEND="$(CACHE_BACKEND)" PUSH=1 \
		./build/ingress-image.sh "$(VERSION)"
	@echo "Pushed multi-arch ($(INGRESS_BUILD_PLATFORMS)): $(HARBOR_INGRESS_IMAGE):$(VERSION)"

# Helm: deploy ingress controller (uses current shell kube context, e.g. KubeLens terminal)
HELM_RELEASE ?= pertisk-proxy-ingress
HELM_NAMESPACE ?= pertisk-proxy
HELM_INGRESS_VALUES ?= deploy/helm/pertisk-ingress/values.yaml
# Cloud deploy: multi-arch by default (amd64 + arm64). Override e.g. DEPLOY_PLATFORMS=linux/amd64.
DEPLOY_PLATFORMS ?= linux/amd64,linux/arm64
deploy-ingress-helm:
	helm upgrade --install $(HELM_RELEASE) deploy/helm/pertisk-ingress \
		-n $(HELM_NAMESPACE) --create-namespace \
		-f $(HELM_INGRESS_VALUES) --set image.tag=$(VERSION)

# Build multi-arch image, push, and deploy with matching tag
deploy-ingress: docker-ingress-multi deploy-ingress-helm
	@echo "Done. $(HELM_RELEASE) deployed with $(HARBOR_INGRESS_IMAGE):$(VERSION)"

# Cloud deploy (Hetzner floating-IP LB + IngressClass mode). See deploy/cloud.sh.
# make deploy-cloud VERSION=1.0.0
# REPLICA_COUNT=1 VERSION=1.0.0 make deploy-cloud   # pin QUIC for HTTP/3 benchmarks
deploy-cloud:
	chmod +x deploy/cloud.sh
	VERSION="$(VERSION)" NAMESPACE="$(HELM_NAMESPACE)" RELEASE_NAME="$(HELM_RELEASE)" \
		DEPLOY_PLATFORMS="$(DEPLOY_PLATFORMS)" ./deploy/cloud.sh

# Talos 285h cluster deploy. See deploy/285h.sh.
# make deploy-285h VERSION=1.0.0
# REPLICA_COUNT=1 VERSION=1.0.0 make deploy-285h
deploy-285h:
	chmod +x deploy/285h.sh
	VERSION="$(VERSION)" NAMESPACE="$(HELM_NAMESPACE)" RELEASE_NAME="$(HELM_RELEASE)" \
		DEPLOY_PLATFORMS="$(DEPLOY_PLATFORMS)" ./deploy/285h.sh

# Talos orion cluster deploy. See deploy/orion.sh.
# make deploy-orion VERSION=1.0.0
# REPLICA_COUNT=1 VERSION=1.0.0 make deploy-orion
deploy-orion:
	chmod +x deploy/orion.sh
	VERSION="$(VERSION)" NAMESPACE="$(HELM_NAMESPACE)" RELEASE_NAME="$(HELM_RELEASE)" \
		DEPLOY_PLATFORMS="$(DEPLOY_PLATFORMS)" ./deploy/orion.sh

# Remove legacy pertisk-rproxy ingress release (ClusterRole name collision with release "pertisk-ingress")
uninstall-legacy-ingress-helm:
	helm uninstall pertisk-ingress -n pertisk-rproxy 2>/dev/null || true
	@echo "If ClusterRole pertisk-ingress remains, delete manually: kubectl delete clusterrole pertisk-ingress clusterrolebinding pertisk-ingress"

# --- Deploy (build package + install on remote host) ---
# Primary: make deploy DEPLOY_HOST=user@host VERSION=0.1.0
# Example:  make deploy-rpm DEPLOY_HOST=user@proxy.example.com VERSION=0.2.26
#   (DEPLOY_ARCH=auto detects aarch64 via SSH; override with DEPLOY_ARCH=amd64|arm64)
# Or:      make deploy-deb DEPLOY_HOST=user@host
#          make deploy-rpm DEPLOY_HOST=user@host
#          make deploy-rpm-arm64 DEPLOY_HOST=user@host VERSION=0.2.26

deploy:
	@$(MAKE) deploy-package DEPLOY_HOST="$(DEPLOY_HOST)" REMOTE_HOST="$(REMOTE_HOST)" \
		DEPLOY_ARCH="$(DEPLOY_ARCH)" DEPLOY_PKG="$(DEPLOY_PKG)" \
		DEPLOY_SSH_OPTS="$(DEPLOY_SSH_OPTS)" VERSION="$(VERSION)" \
		PACKAGE_BUILD="$(PACKAGE_BUILD)" PACKAGE_CLEAN="$(PACKAGE_CLEAN)"

deploy-package:
	@host="$(DEPLOY_HOST)"; \
	if [ -z "$$host" ] && [ -n "$(REMOTE_HOST)" ]; then \
		host="$(REMOTE_USER)@$(REMOTE_HOST)"; \
	fi; \
	if [ -z "$$host" ]; then \
		echo "DEPLOY_HOST is required. Usage: make deploy DEPLOY_HOST=user@host VERSION=0.1.0"; \
		exit 1; \
	fi; \
	chmod +x build/deploy-remote.sh; \
	DEPLOY_HOST="$$host" DEPLOY_ARCH="$(DEPLOY_ARCH)" DEPLOY_BIN="$(DEPLOY_BIN)" \
		DEPLOY_PKG="$(DEPLOY_PKG)" DEPLOY_SSH_OPTS="$(DEPLOY_SSH_OPTS)" VERSION="$(VERSION)" \
		PACKAGE_BUILD="$(PACKAGE_BUILD)" PACKAGE_CLEAN="$(PACKAGE_CLEAN)" \
		./build/deploy-remote.sh

deploy-package-ingress:
	$(MAKE) deploy-package DEPLOY_HOST="$(DEPLOY_HOST)" DEPLOY_BIN=pertisk-proxy-ingress

deploy-remote: deploy-package

deploy-deb:
	chmod +x build/deploy-deb.sh
	DEPLOY_HOST="$(DEPLOY_HOST)" REMOTE_HOST="$(REMOTE_HOST)" REMOTE_USER="$(REMOTE_USER)" \
		VERSION="$(VERSION)" PACKAGE_NAME="$(PACKAGE_NAME)" \
		REMOTE_PATH="$(REMOTE_PATH)" PACKAGE_CLEAN="$(PACKAGE_CLEAN)" \
		PACKAGE_BUILD="$(PACKAGE_BUILD)" DEPLOY_ARCH="$(DEPLOY_ARCH)" \
		DEPLOY_SSH_OPTS="$(DEPLOY_SSH_OPTS)" \
		./build/deploy-deb.sh

deploy-deb-ingress:
	$(MAKE) deploy-deb PACKAGE_NAME=pertisk-proxy-ingress

deploy-deb-arm:
	$(MAKE) deploy-deb DEPLOY_ARCH=arm64

deploy-rpm:
	chmod +x build/deploy-rpm.sh
	DEPLOY_HOST="$(DEPLOY_HOST)" REMOTE_HOST="$(REMOTE_HOST)" REMOTE_USER="$(REMOTE_USER)" \
		VERSION="$(VERSION)" PACKAGE_NAME="$(PACKAGE_NAME)" \
		REMOTE_PATH="$(REMOTE_PATH)" PACKAGE_CLEAN="$(PACKAGE_CLEAN)" \
		PACKAGE_BUILD="$(PACKAGE_BUILD)" DEPLOY_ARCH="$(DEPLOY_ARCH)" \
		DEPLOY_SSH_OPTS="$(DEPLOY_SSH_OPTS)" \
		./build/deploy-rpm.sh

deploy-rpm-ingress:
	$(MAKE) deploy-rpm PACKAGE_NAME=pertisk-proxy-ingress

deploy-rpm-arm deploy-rpm-arm64:
	$(MAKE) deploy-rpm DEPLOY_ARCH=arm64

# Delete a tag (local and remote).
delete-tag:
ifndef TAG
	$(error TAG is not set. Usage: make delete-tag TAG=v1.0.0)
endif
	@echo "Deleting tag $(TAG)..."
	git tag -d $(TAG)
	git push origin -d $(TAG)

# Create a new tag.
create-tag:
ifndef TAG
	$(error TAG is not set. Usage: make create-tag TAG=v1.0.0)
endif
	@echo "Creating tag $(TAG)..."
	git tag $(TAG)
	git push origin $(TAG)

# Delete and recreate a tag (force update). Use after amending a release commit.
# Usage: make retag TAG=v1.0.0
retag:
ifndef TAG
	$(error TAG is not set. Usage: make retag TAG=v1.0.0)
endif
	@echo "Recreating tag $(TAG)..."
	@echo "Deleting local tag (if exists)..."
	-git tag -d $(TAG) 2>/dev/null || true
	@echo "Deleting remote tag (if exists)..."
	-git push origin -d $(TAG) 2>/dev/null || true
	@echo "Creating new tag $(TAG)..."
	git tag $(TAG)
	@echo "Pushing tag $(TAG) to origin..."
	git push origin $(TAG)
	@echo "✓ Tag $(TAG) created and pushed successfully"

clean-tag: retag