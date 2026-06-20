.PHONY: build build-ingress run run-ingress test package package-amd64

CARGO ?= cargo
INGRESS_FEATURES ?= ingress
ROUTES_CONFIG ?= ./config/examples/routes.yaml
ENABLE_H3 ?= true
LOG_LEVEL ?= info

build:
	$(CARGO) build --release --bin pertisk-proxy

build-ingress:
	$(CARGO) build --release --bin pertisk-proxy-ingress --features $(INGRESS_FEATURES)

build-all: build build-ingress

# Proxy mode — standalone reverse proxy
run:
	ROUTES_CONFIG=$(ROUTES_CONFIG) ENABLE_H3=$(ENABLE_H3) PERTISK_LOG_LEVEL=$(LOG_LEVEL) $(CARGO) run --bin pertisk-proxy

run-release:
	ROUTES_CONFIG=$(ROUTES_CONFIG) ENABLE_H3=$(ENABLE_H3) PERTISK_LOG_LEVEL=$(LOG_LEVEL) $(CARGO) run --release --bin pertisk-proxy

# Ingress mode — Kubernetes Ingress controller
run-ingress:
	ENABLE_H3=$(ENABLE_H3) PERTISK_LOG_LEVEL=$(LOG_LEVEL) $(CARGO) run --bin pertisk-proxy-ingress --features $(INGRESS_FEATURES)

run-ingress-release:
	ENABLE_H3=$(ENABLE_H3) PERTISK_LOG_LEVEL=$(LOG_LEVEL) $(CARGO) run --release --bin pertisk-proxy-ingress --features $(INGRESS_FEATURES)

test:
	$(CARGO) test --features $(INGRESS_FEATURES)

package-amd64:
	chmod +x build/package.sh
	./build/package.sh amd64

package-arm64:
	chmod +x build/package.sh
	./build/package.sh arm64

package: package-amd64
