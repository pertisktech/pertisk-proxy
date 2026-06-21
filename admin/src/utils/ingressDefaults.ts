const DEFAULT_INGRESS_NAMESPACE_KEY = 'pertisk_default_ingress_namespace';

export function getDefaultIngressNamespace(): string {
  return (localStorage.getItem(DEFAULT_INGRESS_NAMESPACE_KEY) || '').trim();
}

export function setDefaultIngressNamespace(namespace: string): void {
  const trimmed = namespace.trim();
  if (trimmed) {
    localStorage.setItem(DEFAULT_INGRESS_NAMESPACE_KEY, trimmed);
  } else {
    localStorage.removeItem(DEFAULT_INGRESS_NAMESPACE_KEY);
  }
}
