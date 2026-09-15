import {
  Activity,
  Archive,
  Cable,
  DoorOpen,
  FileUp,
  GitBranch,
  Globe,
  LayoutDashboard,
  ListChecks,
  Network,
  ScrollText,
  Server,
  Settings,
  Shield,
  ShieldAlert,
  type LucideIcon,
} from 'lucide-react';

export type NavItem = {
  to: string;
  label: string;
  icon: LucideIcon;
  end?: boolean;
};

export type NavSection = {
  id: string;
  title: string;
  items: NavItem[];
};

export type CreateItem = {
  to: string;
  label: string;
  icon: LucideIcon;
};

export function proxyNavSections(): NavSection[] {
  return [
    {
      id: 'overview',
      title: 'Overview',
      items: [{ to: '/', label: 'Dashboard', icon: LayoutDashboard, end: true }],
    },
    {
      id: 'routing',
      title: 'Routing',
      items: [
        { to: '/sites', label: 'Sites', icon: Globe },
        { to: '/tunnels', label: 'Tunnels', icon: Cable },
      ],
    },
    {
      id: 'security',
      title: 'Security',
      items: [
        { to: '/access-lists', label: 'Access Control', icon: ListChecks },
        { to: '/waf', label: 'WAF', icon: ShieldAlert },
      ],
    },
    {
      id: 'tls',
      title: 'TLS & DNS',
      items: [
        { to: '/certificates', label: 'Certificates', icon: Shield },
        { to: '/dns-providers', label: 'DNS Providers', icon: Server },
      ],
    },
    {
      id: 'observe',
      title: 'Observe',
      items: [
        { to: '/logs', label: 'Logs', icon: ScrollText },
        { to: '/metrics', label: 'Metrics', icon: Activity },
      ],
    },
    {
      id: 'system',
      title: 'System',
      items: [
        { to: '/backup', label: 'Backup', icon: Archive },
        { to: '/settings', label: 'Settings', icon: Settings },
      ],
    },
  ];
}

export function ingressNavSections(gatewayApiEnabled: boolean): NavSection[] {
  const routing: NavItem[] = [{ to: '/sites/ingress', label: 'Ingress', icon: Network }];
  if (gatewayApiEnabled) {
    routing.push(
      { to: '/sites/gateway/gateways', label: 'Gateways', icon: DoorOpen },
      { to: '/sites/gateway/sites', label: 'HTTP Routes', icon: GitBranch },
    );
  }

  return [
    {
      id: 'overview',
      title: 'Overview',
      items: [{ to: '/', label: 'Dashboard', icon: LayoutDashboard, end: true }],
    },
    { id: 'routing', title: 'Routing', items: routing },
    {
      id: 'security',
      title: 'Security',
      items: [
        { to: '/access-lists', label: 'Access Control', icon: ListChecks },
        { to: '/waf', label: 'WAF', icon: ShieldAlert },
      ],
    },
    {
      id: 'tls',
      title: 'TLS',
      items: [{ to: '/certificates', label: 'Certificates', icon: Shield }],
    },
    {
      id: 'observe',
      title: 'Observe',
      items: [
        { to: '/logs', label: 'Logs', icon: ScrollText },
        { to: '/metrics', label: 'Metrics', icon: Activity },
      ],
    },
    {
      id: 'system',
      title: 'System',
      items: [
        { to: '/backup', label: 'Backup', icon: Archive },
        { to: '/settings', label: 'Settings', icon: Settings },
      ],
    },
  ];
}

export function createMenuItems(mode: 'proxy' | 'ingress' | undefined): CreateItem[] {
  const siteTo = mode === 'ingress' ? '/sites/ingress?new=1' : '/sites?new=1';
  const items: CreateItem[] = [{ to: siteTo, label: 'Site', icon: Globe }];
  if (mode !== 'ingress') {
    items.push({ to: '/dns-providers?new=1', label: 'DNS provider', icon: Server });
  }
  items.push(
    { to: '/certificates?import=1', label: 'Import certificate', icon: FileUp },
    { to: '/access-lists?new=1', label: 'Access Control', icon: ListChecks },
    { to: '/waf?new=1', label: 'WAF', icon: ShieldAlert },
  );
  return items;
}

export function resolveNavTitle(pathname: string, sections: NavSection[]): string {
  const items = sections.flatMap((s) => s.items);
  const match = items
    .filter((n) => (n.end ? pathname === n.to : pathname === n.to || pathname.startsWith(`${n.to}/`)))
    .sort((a, b) => b.to.length - a.to.length)[0];
  if (match) return match.label;
  if (pathname.startsWith('/profile')) return 'Profile';
  if (pathname.startsWith('/route-map')) return 'Route map';
  return 'Admin';
}
