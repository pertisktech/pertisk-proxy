#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

if [[ -z "${VERSION:-}" ]]; then
  VERSION="$(git describe --tags --always 2>/dev/null | sed 's/^v//' || true)"
  VERSION="${VERSION:-0.0.0}"
fi

NAMESPACE="${NAMESPACE:-pertisk-proxy}"
RELEASE_NAME="${RELEASE_NAME:-pertisk-proxy-ingress}"
CHART_PATH="${CHART_PATH:-./deploy/helm/pertisk-ingress}"
CLOUD_VALUES="${CLOUD_VALUES:-./deploy/helm/pertisk-ingress/cloud/values.yaml}"
CRD_DIR="${CRD_DIR:-./deploy/helm/pertisk-ingress/crds}"
CRD_TIMEOUT="${CRD_TIMEOUT:-180s}"
APPLY_CRDS="${APPLY_CRDS:-1}"
WAIT_FOR_CRDS="${WAIT_FOR_CRDS:-0}"
ADMIN_HOST="${ADMIN_HOST:-admin.example.com}"
ADMIN_TLS_SECRET="${ADMIN_TLS_SECRET:-admin-tls}"
HELM_TIMEOUT="${HELM_TIMEOUT:-20m}"
# Build both architectures so arm64 clusters (e.g. Hetzner CAX) and amd64 nodes both work.
DEPLOY_PLATFORMS="${DEPLOY_PLATFORMS:-linux/amd64,linux/arm64}"

# Optional Auth0 SSO — set AUTH0_DOMAIN / AUTH0_CLIENT_ID / AUTH0_AUDIENCE to enable.
AUTH0_DOMAIN="${AUTH0_DOMAIN:-}"
AUTH0_CLIENT_ID="${AUTH0_CLIENT_ID:-}"
AUTH0_AUDIENCE="${AUTH0_AUDIENCE:-}"
AUTH_PASSWORD="${AUTH_PASSWORD:-changeme}"

# Hetzner / pertisk floating-IP controller (service.annotations on LoadBalancer).
FLOATING_IP_ENABLED="${FLOATING_IP_ENABLED:-true}"
FLOATING_IP_FAMILY="${FLOATING_IP_FAMILY:-dual-stack}"
FLOATING_IP_HOME_LOCATION="${FLOATING_IP_HOME_LOCATION:-nbg1}"

OPTIONAL_SET_ARGS=()
if [[ -n "${AUTH_SIGNING_SECRET:-}" ]]; then
  OPTIONAL_SET_ARGS+=(--set "auth.signingSecret=${AUTH_SIGNING_SECRET}")
fi
# HTTP/3 benchmarks: REPLICA_COUNT=1 pins QUIC to one pod (see cloud/values.yaml).
if [[ -n "${REPLICA_COUNT:-}" ]]; then
  OPTIONAL_SET_ARGS+=(--set "replicaCount=${REPLICA_COUNT}")
  OPTIONAL_SET_ARGS+=(--set "autoscaling.minReplicas=${REPLICA_COUNT}")
  if [[ "${REPLICA_COUNT}" == "1" ]]; then
    OPTIONAL_SET_ARGS+=(--set "autoscaling.maxReplicas=1")
  fi
fi

echo "Deploying ${RELEASE_NAME} version ${VERSION} to namespace ${NAMESPACE}"
echo "Building ingress image first (platforms: ${DEPLOY_PLATFORMS})..."
make docker-ingress-multi VERSION="${VERSION}" INGRESS_BUILD_PLATFORMS="${DEPLOY_PLATFORMS}"

if [[ "${APPLY_CRDS}" == "1" ]]; then
  echo "Applying ingress CRDs from ${CRD_DIR}..."
  if ! kubectl apply -f "${CRD_DIR}"; then
    echo "CRD apply failed (likely OpenAPI validation fetch/parsing issue). Retrying with --validate=false..."
    if ! kubectl apply --validate=false -f "${CRD_DIR}"; then
      echo "CRD apply still failing during server-side read. Falling back to create-only mode..."
      crd_files=("${CRD_DIR}"/*.yaml)
      for crd_file in "${crd_files[@]}"; do
        if ! kubectl create --validate=false -f "${crd_file}" 2>/tmp/pertisk-crd-create.err; then
          if grep -qi "AlreadyExists" /tmp/pertisk-crd-create.err; then
            echo "CRD already exists: ${crd_file}"
          else
            cat /tmp/pertisk-crd-create.err >&2
            rm -f /tmp/pertisk-crd-create.err
            exit 1
          fi
        fi
      done
      rm -f /tmp/pertisk-crd-create.err
    fi
  fi
  if [[ "${WAIT_FOR_CRDS}" == "1" ]]; then
    echo "Waiting for CRDs to become Established..."
    kubectl wait --for=condition=Established --timeout="${CRD_TIMEOUT}" \
      crd/pertiskbackends.proxy.pertisk.tech \
      crd/pertiskingresses.proxy.pertisk.tech
  fi
fi

helm upgrade --install "${RELEASE_NAME}" "${CHART_PATH}" \
  -n "${NAMESPACE}" \
  --create-namespace \
  --skip-crds \
  --timeout "${HELM_TIMEOUT}" \
  -f "${CLOUD_VALUES}" \
  --set installCRDs=false \
  --set image.tag="${VERSION}" \
  --set image.pullPolicy=Always \
  --set ingressClassName=pertisk-proxy \
  --set auth.username=admin \
  --set auth.password="${AUTH_PASSWORD}" \
  --set auth0.domain="${AUTH0_DOMAIN}" \
  --set auth0.clientId="${AUTH0_CLIENT_ID}" \
  --set auth0.audience="${AUTH0_AUDIENCE}" \
  --set adminIngress.enabled=true \
  --set adminIngress.host="${ADMIN_HOST}" \
  --set adminIngress.tlsSecretName="${ADMIN_TLS_SECRET}" \
  --set gatewayApi.enabled=false \
  --set gatewayClassResource.enabled=false \
  --set ingressClassResource.enabled=true \
  --set-string service.annotations."pertisk\.tech/floating-ip-enabled"="${FLOATING_IP_ENABLED}" \
  --set-string service.annotations."pertisk\.tech/floating-ip-family"="${FLOATING_IP_FAMILY}" \
  --set-string service.annotations."pertisk\.tech/floating-ip-home-location"="${FLOATING_IP_HOME_LOCATION}" \
  --force-conflicts \
  ${OPTIONAL_SET_ARGS+"${OPTIONAL_SET_ARGS[@]}"}

echo "Done. ${RELEASE_NAME} deployed with image tag ${VERSION}"
