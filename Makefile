.PHONY: build build-ingress build-all run run-release run-ingress run-ingress-release \
	test check package package-clean package-amd64 package-arm64 package-deb package-rpm \
	package-proxy package-ingress release release-amd release-arm \
	deploy-package deploy-package-ingress deploy-remote \
	deploy-deb deploy-deb-ingress deploy-rpm deploy-rpm-ingress apply-ingress-rbac \
	install-admin admin-dist dev dev-serve dev-admin dev-stop

CARGO ?= cargo
INGRESS_FEATURES ?= ingress
ROUTES_CONFIG ?= ./config/examples/routes.yaml
ENABLE_H3 ?= true
PROXY_MODE ?= performance
LOG_LEVEL ?= info

# Dev listen addresses (DNS-ready: 80 + 443/tcp + 443/udp). On macOS use: sudo make dev
DEV_LISTEN_HTTP ?= 0.0.0.0:80
DEV_LISTEN_HTTPS ?= 0.0.0.0:443
DEV_LISTEN_H3_UDP ?= [::]:443
DEV_MANAGEMENT_ADDR ?= 127.0.0.1:9080

VERSION ?= $(shell git describe --tags --always 2>/dev/null | sed 's/^v//' || echo "0.1.0")
PACKAGE_TARGET ?= all
BUILDER_NAME ?= pertisk-proxy-package
CACHE_DIR ?= .buildx-cache/release

# Remote deploy (make deploy-package DEPLOY_HOST=user@host)
DEPLOY_HOST ?=
DEPLOY_ARCH ?= amd64
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

# Kubernetes RBAC apply (one-time, before ingress DEB/RPM deploy)
KUBECONFIG ?=
K8S_KUBECONFIG ?=

build:
	$(CARGO) build --release --bin pertisk-proxy --features admin

build-ingress:
	$(CARGO) build --release --bin pertisk-proxy-ingress --features $(INGRESS_FEATURES)

build-all: build build-ingress

check:
	$(CARGO) check --features $(INGRESS_FEATURES)

# Admin UI (React + Vite)
install-admin:
	cd admin && pnpm install

admin-dist:
	@echo "Checking admin UI build cache..."
	@if [ -d admin/dist ] && [ -z "$$(find admin/src admin/public admin/index.html admin/package.json admin/pnpm-lock.yaml -type f -newer admin/dist 2>/dev/null | head -n 1)" ]; then \
		echo "admin/dist is up to date; skipping build."; \
	else \
		echo "Building admin UI..."; \
		if [ ! -d admin/node_modules ]; then $(MAKE) install-admin; fi && (cd admin && pnpm run build); \
	fi

dev-admin:
	cd admin && pnpm dev

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

# Backend + admin Vite dev. Listens 80/443/tcp + 443/udp — use https://admin.amd.thaidevops.co/
# macOS requires root for 80/443: sudo make dev
dev: dev-stop
	ROUTES_CONFIG=$(ROUTES_CONFIG) ENABLE_H3=$(ENABLE_H3) PERTISK_PROXY_MODE=$(PROXY_MODE) \
		PERTISK_LOG_LEVEL=$(LOG_LEVEL) \
		LISTEN_HTTP=$(DEV_LISTEN_HTTP) LISTEN_HTTPS=$(DEV_LISTEN_HTTPS) LISTEN_H3_UDP=$(DEV_LISTEN_H3_UDP) \
		PERTISK_MANAGEMENT_ADDR=$(DEV_MANAGEMENT_ADDR) \
		PERTISK_ADMIN_UI_DEV_ORIGIN=http://127.0.0.1:5173 \
		$(CARGO) watch -i admin -x 'run --bin pertisk-proxy --features admin' & \
	(cd admin && API_PROXY_TARGET=http://$(DEV_MANAGEMENT_ADDR) pnpm dev) & \
	wait

# Backend + admin auto-rebuild; open http://127.0.0.1:9080
dev-serve: admin-dist dev-stop
	ROUTES_CONFIG=$(ROUTES_CONFIG) ENABLE_H3=$(ENABLE_H3) PERTISK_PROXY_MODE=$(PROXY_MODE) \
		PERTISK_LOG_LEVEL=$(LOG_LEVEL) \
		LISTEN_HTTP=$(DEV_LISTEN_HTTP) LISTEN_HTTPS=$(DEV_LISTEN_HTTPS) LISTEN_H3_UDP=$(DEV_LISTEN_H3_UDP) \
		PERTISK_MANAGEMENT_ADDR=$(DEV_MANAGEMENT_ADDR) \
		$(CARGO) watch -i admin -x 'run --bin pertisk-proxy --features admin' & \
	(cd admin && pnpm run build:watch) & \
	wait

# Proxy mode — standalone reverse proxy
run:
	ROUTES_CONFIG=$(ROUTES_CONFIG) ENABLE_H3=$(ENABLE_H3) PERTISK_PROXY_MODE=$(PROXY_MODE) PERTISK_LOG_LEVEL=$(LOG_LEVEL) $(CARGO) run --bin pertisk-proxy

run-release: admin-dist
	ROUTES_CONFIG=$(ROUTES_CONFIG) ENABLE_H3=$(ENABLE_H3) PERTISK_PROXY_MODE=$(PROXY_MODE) PERTISK_LOG_LEVEL=$(LOG_LEVEL) $(CARGO) run --release --bin pertisk-proxy

# Ingress mode — Kubernetes Ingress controller (uses current kubeconfig)
run-ingress:
	ENABLE_H3=$(ENABLE_H3) PERTISK_LOG_LEVEL=$(LOG_LEVEL) $(CARGO) run --bin pertisk-proxy-ingress --features $(INGRESS_FEATURES)

run-ingress-release:
	ENABLE_H3=$(ENABLE_H3) PERTISK_LOG_LEVEL=$(LOG_LEVEL) $(CARGO) run --release --bin pertisk-proxy-ingress --features $(INGRESS_FEATURES)

test:
	$(CARGO) test --features $(INGRESS_FEATURES)

# --- Packaging: DEB + RPM + tarball (Docker cross-build on macOS) → release/ ---
# Requires: docker (buildx cross-compile + fpm only; no runtime images).
# make package              — amd64 + arm64, both binaries
# make package-amd64        — amd64 only
# make package-proxy        — proxy binary only
# make package-ingress      — ingress binary only (Kubernetes controller)

package-clean:
	rm -f pertisk-proxy-linux-amd64 pertisk-proxy-linux-arm64 \
		pertisk-proxy-ingress-linux-amd64 pertisk-proxy-ingress-linux-arm64
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

release-amd:
	$(MAKE) package-clean
	$(MAKE) package-amd64 VERSION=$(VERSION)

release-arm:
	$(MAKE) package-clean
	$(MAKE) package-arm64 VERSION=$(VERSION)

release-proxy:
	$(MAKE) package-clean
	$(MAKE) package-proxy VERSION=$(VERSION)

release-ingress:
	$(MAKE) package-clean
	$(MAKE) package-ingress VERSION=$(VERSION)

# One-time: IngressClass + ClusterRole for external systemd ingress controller
apply-ingress-rbac:
	@if [ -n "$(K8S_KUBECONFIG)" ]; then \
		KUBECONFIG="$(K8S_KUBECONFIG)" kubectl apply -f deploy/kubernetes-rbac.yaml; \
	elif [ -n "$(KUBECONFIG)" ]; then \
		KUBECONFIG="$(KUBECONFIG)" kubectl apply -f deploy/kubernetes-rbac.yaml; \
	else \
		kubectl apply -f deploy/kubernetes-rbac.yaml; \
	fi

# Remote install DEB/RPM over SSH
deploy-package:
ifndef DEPLOY_HOST
	$(error DEPLOY_HOST is required. Usage: make deploy-package DEPLOY_HOST=user@host)
endif
	chmod +x build/deploy-remote.sh
	DEPLOY_HOST="$(DEPLOY_HOST)" DEPLOY_ARCH="$(DEPLOY_ARCH)" DEPLOY_BIN="$(DEPLOY_BIN)" \
		DEPLOY_PKG="$(DEPLOY_PKG)" DEPLOY_SSH_OPTS="$(DEPLOY_SSH_OPTS)" VERSION="$(VERSION)" \
		./build/deploy-remote.sh

deploy-package-ingress:
	$(MAKE) deploy-package DEPLOY_HOST="$(DEPLOY_HOST)" DEPLOY_BIN=pertisk-proxy-ingress

deploy-remote: deploy-package

# Build package + deploy DEB/RPM (primary deployment path)
deploy-deb:
	chmod +x build/deploy-deb.sh
	REMOTE_HOST="$(REMOTE_HOST)" REMOTE_USER="$(REMOTE_USER)" VERSION="$(VERSION)" \
		PACKAGE_NAME="$(PACKAGE_NAME)" \
		REMOTE_PATH="$(REMOTE_PATH)" PACKAGE_CLEAN="$(PACKAGE_CLEAN)" \
		PACKAGE_BUILD="$(PACKAGE_BUILD)" ./build/deploy-deb.sh

deploy-deb-ingress:
	$(MAKE) deploy-deb PACKAGE_NAME=pertisk-proxy-ingress

deploy-rpm:
	chmod +x build/deploy-rpm.sh
	REMOTE_HOST="$(REMOTE_HOST)" REMOTE_USER="$(REMOTE_USER)" VERSION="$(VERSION)" \
		PACKAGE_NAME="$(PACKAGE_NAME)" \
		REMOTE_PATH="$(REMOTE_PATH)" PACKAGE_CLEAN="$(PACKAGE_CLEAN)" \
		PACKAGE_BUILD="$(PACKAGE_BUILD)" ./build/deploy-rpm.sh

deploy-rpm-ingress:
	$(MAKE) deploy-rpm PACKAGE_NAME=pertisk-proxy-ingress
