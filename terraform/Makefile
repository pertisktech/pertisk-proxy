VERSION ?= 0.1.0
OS_ARCH := $(shell go env GOOS)_$(shell go env GOARCH)
BIN := terraform-provider-pertisk-proxy
ORG := pertisktech
PLUGIN_DIR := $(HOME)/.terraform.d/plugins/registry.terraform.io/$(ORG)/pertisk-proxy/$(VERSION)/$(OS_ARCH)

.PHONY: build install tidy test fmt release publish sync-provider-repo

tidy:
	go mod tidy

build: tidy
	go build -ldflags="-X main.version=$(VERSION)" -o bin/$(BIN) .

install: build
	mkdir -p "$(PLUGIN_DIR)"
	cp bin/$(BIN) "$(PLUGIN_DIR)/$(BIN)_v$(VERSION)"
	@echo "installed → $(PLUGIN_DIR)/$(BIN)_v$(VERSION)"

fmt:
	gofmt -w .

test:
	go test ./...

release:
	VERSION=$(VERSION) bash scripts/release.sh

publish: release
	VERSION=$(VERSION) ORG=$(ORG) bash scripts/publish-hcp.sh

# Mirror terraform/ → github.com/pertisktech/terraform-provider-pertisk-proxy (public Registry).
sync-provider-repo:
	bash scripts/sync-provider-repo.sh
