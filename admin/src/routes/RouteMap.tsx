import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  ReactFlow,
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  useNodesState,
  useEdgesState,
  MarkerType,
  Handle,
  Position,
  Panel,
  type Node,
  type Edge,
  type NodeProps,
  type ReactFlowInstance,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { toPng } from 'html-to-image';
import { useNavigate } from 'react-router-dom';
import {
  ChevronDown,
  ChevronUp,
  Globe,
  GitBranch,
  Loader,
  Map as MapIcon,
  Maximize2,
  Minimize2,
  Minus,
  Network,
  Plus,
  RefreshCw,
  Server,
  Shield,
  Upload,
  X,
} from 'lucide-react';
import { toast } from 'sonner';
import { api, type ProxyConfig, type RouteView } from '@/api/client';
import { useMode } from '@/context/ModeContext';
import { cn } from '@/utils';

type RouteMapNode = {
  id: string;
  kind: string;
  name: string;
  detail?: string;
  status: string;
};

type RouteMapEdge = {
  source: string;
  target: string;
  edge_type: string;
};

type RouteMapData = {
  nodes: RouteMapNode[];
  edges: RouteMapEdge[];
};

type KindConfig = {
  color: string;
  bg: string;
  border: string;
  icon: typeof Globe;
  navPath?: string;
};

type FlowNodeData = RouteMapNode & {
  collapsed: boolean;
  canToggleCollapse: boolean;
  onToggleCollapse: (nodeId: string) => void;
};

const KIND_CONFIG: Record<string, KindConfig> = {
  Certificate: {
    color: 'text-red-r1',
    bg: 'bg-red-r1/10',
    border: 'border-red-r1/30',
    icon: Shield,
    navPath: '/certificates',
  },
  Site: {
    color: 'text-primary',
    bg: 'bg-primary/10',
    border: 'border-primary/30',
    icon: Globe,
  },
  Route: {
    color: 'text-blue-b1',
    bg: 'bg-blue-b1/10',
    border: 'border-blue-b1/30',
    icon: GitBranch,
  },
  Backend: {
    color: 'text-green-g1',
    bg: 'bg-green-g1/10',
    border: 'border-green-g1/30',
    icon: Network,
  },
  Upstream: {
    color: 'text-yellow-y1',
    bg: 'bg-yellow-y1/10',
    border: 'border-yellow-y1/30',
    icon: Server,
  },
};

const STATUS_DOT: Record<string, string> = {
  active: 'bg-green-g1',
  ready: 'bg-green-g1',
  tls: 'bg-blue-b1',
  unknown: 'bg-muted',
};

const MINIMAP_COLOR: Record<string, string> = {
  Certificate: '#ef4444',
  Site: '#6366f1',
  Route: '#3b82f6',
  Backend: '#22c55e',
  Upstream: '#eab308',
};

const KIND_COL: Record<string, number> = {
  Certificate: 0,
  Site: 1,
  Route: 2,
  Backend: 3,
  Upstream: 4,
};

const NODE_WIDTH = 192;
const COL_GAP = 272;
const ROW_GAP = 86;
const REFRESH_INTERVAL = 15_000;

function computeVisibleNodeIds(
  apiNodes: RouteMapNode[],
  apiEdges: RouteMapEdge[],
  collapsedNodeIds: Set<string>,
) {
  const nodeIds = new Set(apiNodes.map((node) => node.id));
  const outgoing = new Map<string, string[]>();
  const incomingCount = new Map<string, number>();

  apiNodes.forEach((node) => {
    outgoing.set(node.id, []);
    incomingCount.set(node.id, 0);
  });

  apiEdges.forEach((edge) => {
    if (!nodeIds.has(edge.source) || !nodeIds.has(edge.target)) return;
    outgoing.get(edge.source)?.push(edge.target);
    incomingCount.set(edge.target, (incomingCount.get(edge.target) ?? 0) + 1);
  });

  const visitFrom = (startId: string, visibleNodeIds: Set<string>) => {
    const stack = [startId];
    while (stack.length > 0) {
      const currentId = stack.pop()!;
      if (visibleNodeIds.has(currentId)) continue;
      visibleNodeIds.add(currentId);
      if (collapsedNodeIds.has(currentId)) continue;
      const children = outgoing.get(currentId) ?? [];
      for (let index = children.length - 1; index >= 0; index -= 1) {
        stack.push(children[index]);
      }
    }
  };

  const visibleNodeIds = new Set<string>();
  const rootIds = apiNodes
    .filter((node) => (incomingCount.get(node.id) ?? 0) === 0)
    .map((node) => node.id);

  (rootIds.length > 0 ? rootIds : apiNodes.map((node) => node.id)).forEach((nodeId) => {
    visitFrom(nodeId, visibleNodeIds);
  });

  if (visibleNodeIds.size < apiNodes.length) {
    apiNodes.forEach((node) => {
      if (!visibleNodeIds.has(node.id)) visitFrom(node.id, visibleNodeIds);
    });
  }

  return { visibleNodeIds, outgoing };
}

function computeLayout(
  apiNodes: RouteMapNode[],
  apiEdges: RouteMapEdge[],
  getNodeData: (node: RouteMapNode) => FlowNodeData,
) {
  const colNodes: Record<number, RouteMapNode[]> = {};
  for (const n of apiNodes) {
    const col = KIND_COL[n.kind] ?? 3;
    if (!colNodes[col]) colNodes[col] = [];
    colNodes[col].push(n);
  }

  const compareNodes = (a: RouteMapNode, b: RouteMapNode) => {
    const da = a.detail ?? '';
    const db = b.detail ?? '';
    return da !== db ? da.localeCompare(db) : a.name.localeCompare(b.name);
  };

  const incoming = new Map<string, string[]>();
  const outgoing = new Map<string, string[]>();
  apiNodes.forEach((node) => {
    incoming.set(node.id, []);
    outgoing.set(node.id, []);
  });
  apiEdges.forEach((edge) => {
    incoming.get(edge.target)?.push(edge.source);
    outgoing.get(edge.source)?.push(edge.target);
  });

  const sortedCols = Object.keys(colNodes)
    .map(Number)
    .sort((a, b) => a - b);

  sortedCols.forEach((col) => {
    colNodes[col].sort(compareNodes);
  });

  const orderIndex = new Map<string, number>();
  const refreshOrderIndex = () => {
    sortedCols.forEach((col) => {
      colNodes[col].forEach((node, index) => {
        orderIndex.set(node.id, index);
      });
    });
  };

  const getBarycenter = (neighborIds: string[]) => {
    const positions = neighborIds
      .map((id) => orderIndex.get(id))
      .filter((value): value is number => value !== undefined);
    if (positions.length === 0) return Number.POSITIVE_INFINITY;
    return positions.reduce((sum, value) => sum + value, 0) / positions.length;
  };

  refreshOrderIndex();
  for (let pass = 0; pass < 4; pass += 1) {
    for (let i = 1; i < sortedCols.length; i += 1) {
      const col = sortedCols[i];
      colNodes[col].sort((a, b) => {
        const baryA = getBarycenter(incoming.get(a.id) ?? []);
        const baryB = getBarycenter(incoming.get(b.id) ?? []);
        if (baryA === baryB) return compareNodes(a, b);
        return baryA - baryB;
      });
      refreshOrderIndex();
    }
    for (let i = sortedCols.length - 2; i >= 0; i -= 1) {
      const col = sortedCols[i];
      colNodes[col].sort((a, b) => {
        const baryA = getBarycenter(outgoing.get(a.id) ?? []);
        const baryB = getBarycenter(outgoing.get(b.id) ?? []);
        if (baryA === baryB) return compareNodes(a, b);
        return baryA - baryB;
      });
      refreshOrderIndex();
    }
  }

  const posMap: Record<string, { x: number; y: number }> = {};
  for (const [colStr, nodes] of Object.entries(colNodes)) {
    const col = Number(colStr);
    nodes.forEach((n, i) => {
      posMap[n.id] = { x: col * COL_GAP, y: i * ROW_GAP };
    });
  }

  const rfNodes: Node<FlowNodeData>[] = apiNodes.map((n) => ({
    id: n.id,
    type: 'routeNode',
    position: posMap[n.id] ?? { x: 0, y: 0 },
    data: getNodeData(n),
  }));

  const EDGE_STYLE: Record<string, { stroke: string; dash?: string; animated?: boolean }> = {
    covers: { stroke: '#ef4444', dash: '4 3' },
    routes: { stroke: '#f97316', dash: '5 3' },
    uses: { stroke: '#22c55e', animated: true },
    forwards: { stroke: '#3b82f6' },
    owns: { stroke: 'var(--color-border)' },
  };

  const kindFromId = (id: string) => {
    const prefix = id.split('/')[0];
    const map: Record<string, string> = {
      cert: 'Certificate',
      site: 'Site',
      route: 'Route',
      backend: 'Backend',
      upstream: 'Upstream',
    };
    return map[prefix] ?? '';
  };

  const rfEdges: Edge[] = apiEdges.map((e) => {
    const style = EDGE_STYLE[e.edge_type] ?? EDGE_STYLE.owns;
    const sourceCol = KIND_COL[kindFromId(e.source)] ?? 0;
    const targetCol = KIND_COL[kindFromId(e.target)] ?? 0;
    const edgeType = Math.abs(targetCol - sourceCol) > 1 ? 'default' : 'smoothstep';
    return {
      id: `${e.source}--${e.target}--${e.edge_type}`,
      source: e.source,
      target: e.target,
      type: edgeType,
      animated: style.animated ?? false,
      style: {
        stroke: style.stroke,
        strokeDasharray: style.dash,
        strokeWidth: 1.5,
      },
      markerEnd: {
        type: MarkerType.ArrowClosed,
        color: style.stroke,
        width: 14,
        height: 14,
      },
    };
  });

  return { rfNodes, rfEdges };
}

function upsertNode(map: Map<string, RouteMapNode>, node: RouteMapNode) {
  if (!map.has(node.id)) map.set(node.id, node);
}

function routeKey(host: string, path: string): string {
  return `${host.trim().toLowerCase()}|${path || '/'}`;
}

function buildRouteMapData(routes: RouteView[], config: ProxyConfig | null): RouteMapData {
  const nodes = new Map<string, RouteMapNode>();
  const edges: RouteMapEdge[] = [];
  const edgeKeys = new Set<string>();

  const pushEdge = (source: string, target: string, edge_type: string) => {
    const key = `${source}|${target}|${edge_type}`;
    if (edgeKeys.has(key)) return;
    edgeKeys.add(key);
    edges.push({ source, target, edge_type });
  };

  const hostToBackend = new Map<string, string>();
  const backendUpstreams = new Map<string, string[]>();
  /** Per-route upstream override (host|path → addr). These skip the Backend node. */
  const routeUpstreamOverride = new Map<string, string>();

  if (config) {
    for (const site of config.sites ?? []) {
      const host = site.host?.trim();
      if (!host) continue;
      if (site.backend) hostToBackend.set(host.toLowerCase(), site.backend);
      for (const r of site.routes ?? []) {
        const override = r.upstream?.trim();
        if (!override) continue;
        routeUpstreamOverride.set(routeKey(host, r.path || '/'), override);
      }
    }
    for (const be of config.backends ?? []) {
      backendUpstreams.set(
        be.name,
        (be.upstreams ?? []).map((u) => u.addr).filter(Boolean),
      );
    }
    (config.tls ?? []).forEach((tls, index) => {
      const hosts = tls.hosts ?? [];
      if (hosts.length === 0) return;
      const certId = `cert/${index}-${hosts[0]}`;
      upsertNode(nodes, {
        id: certId,
        kind: 'Certificate',
        name: hosts.join(', '),
        detail: tls.source?.type ?? 'tls',
        status: 'tls',
      });
      for (const host of hosts) {
        const siteId = `site/${host.toLowerCase()}`;
        upsertNode(nodes, {
          id: siteId,
          kind: 'Site',
          name: host,
          status: 'active',
        });
        pushEdge(certId, siteId, 'covers');
      }
    });
  }

  for (const route of routes) {
    const host = route.host?.trim();
    if (!host) continue;
    const path = route.path || '/';
    const pathType = route.path_type || 'prefix';
    const resolvedUpstream = route.upstream?.trim() || '';
    const overrideUpstream = routeUpstreamOverride.get(routeKey(host, path));
    // Prefer config override when present; otherwise use live resolved upstream.
    const directUpstream = overrideUpstream || '';

    const siteId = `site/${host.toLowerCase()}`;
    const routeId = `route/${host.toLowerCase()}${path}`;
    upsertNode(nodes, {
      id: siteId,
      kind: 'Site',
      name: host,
      status: 'active',
    });

    const detailParts = [pathType];
    if (overrideUpstream) detailParts.push('upstream override');
    if (route.middlewares) detailParts.push(`${route.middlewares} mw`);

    upsertNode(nodes, {
      id: routeId,
      kind: 'Route',
      name: path,
      detail: detailParts.join(' · '),
      status: 'active',
    });
    pushEdge(siteId, routeId, 'routes');

    // Per-route upstream override: Route → Upstream (no Backend).
    if (directUpstream) {
      const upstreamId = `upstream/${directUpstream}`;
      upsertNode(nodes, {
        id: upstreamId,
        kind: 'Upstream',
        name: directUpstream,
        detail: 'override',
        status: 'ready',
      });
      pushEdge(routeId, upstreamId, 'forwards');
      continue;
    }

    const backendName = hostToBackend.get(host.toLowerCase());
    if (backendName) {
      const backendId = `backend/${backendName}`;
      upsertNode(nodes, {
        id: backendId,
        kind: 'Backend',
        name: backendName,
        status: 'active',
      });
      pushEdge(routeId, backendId, 'uses');

      const addrs = backendUpstreams.get(backendName) ?? [];
      const targets = addrs.length > 0 ? addrs : resolvedUpstream ? [resolvedUpstream] : [];
      for (const addr of targets) {
        const upstreamId = `upstream/${addr}`;
        upsertNode(nodes, {
          id: upstreamId,
          kind: 'Upstream',
          name: addr,
          status: 'ready',
        });
        pushEdge(backendId, upstreamId, 'owns');
      }
    } else if (resolvedUpstream) {
      const upstreamId = `upstream/${resolvedUpstream}`;
      upsertNode(nodes, {
        id: upstreamId,
        kind: 'Upstream',
        name: resolvedUpstream,
        status: 'ready',
      });
      pushEdge(routeId, upstreamId, 'forwards');
    }
  }

  return { nodes: Array.from(nodes.values()), edges };
}

const RouteNode = memo(({ data, selected }: NodeProps<Node<FlowNodeData>>) => {
  const node = data as FlowNodeData;
  const config = KIND_CONFIG[node.kind] ?? KIND_CONFIG.Site;
  const Icon = config.icon;
  const dotColor = STATUS_DOT[node.status] ?? 'bg-muted';

  return (
    <div
      className={cn(
        'flex items-center gap-2.5 rounded-lg border bg-surface px-3 py-2.5 shadow-sm transition-shadow',
        config.border,
        selected && 'shadow-md ring-2 ring-primary ring-offset-0',
      )}
      style={{ width: NODE_WIDTH }}
    >
      <Handle
        type="target"
        position={Position.Left}
        style={{ background: 'transparent', border: 'none', width: 6, height: 6 }}
      />
      <div className={cn('shrink-0 rounded-md p-1.5', config.bg)}>
        <Icon size={13} className={config.color} />
      </div>
      <div className="min-w-0 flex-1">
        <div className="mb-0.5 flex items-center gap-1.5">
          <span className={cn('text-[9px] font-bold uppercase tracking-wide leading-none', config.color)}>
            {node.kind}
          </span>
          <div className={cn('h-1.5 w-1.5 shrink-0 rounded-full', dotColor)} title={node.status} />
        </div>
        <div className="truncate text-[11px] font-semibold leading-tight text-text">{node.name}</div>
        {node.detail ? (
          <div className="mt-0.5 truncate text-[9px] leading-none text-text-secondary">{node.detail}</div>
        ) : null}
      </div>
      {node.canToggleCollapse ? (
        <button
          type="button"
          className="nodrag nopan flex h-5 w-5 shrink-0 items-center justify-center rounded-md border border-border text-text-secondary transition-colors hover:bg-hover hover:text-text"
          onClick={(event) => {
            event.stopPropagation();
            node.onToggleCollapse(node.id);
          }}
          aria-label={node.collapsed ? `Expand ${node.kind} ${node.name}` : `Collapse ${node.kind} ${node.name}`}
          title={node.collapsed ? 'Expand descendants' : 'Collapse descendants'}
        >
          {node.collapsed ? <Plus size={11} strokeWidth={2.5} /> : <Minus size={11} strokeWidth={2.5} />}
        </button>
      ) : null}
      <Handle
        type="source"
        position={Position.Right}
        style={{ background: 'transparent', border: 'none', width: 6, height: 6 }}
      />
    </div>
  );
});
RouteNode.displayName = 'RouteNode';

const nodeTypes = { routeNode: RouteNode };

function DetailDrawer({
  node,
  sitesPath,
  onClose,
}: {
  node: RouteMapNode;
  sitesPath: string;
  onClose: () => void;
}) {
  const navigate = useNavigate();
  const config = KIND_CONFIG[node.kind] ?? KIND_CONFIG.Site;
  const Icon = config.icon;
  const dotColor = STATUS_DOT[node.status] ?? 'bg-muted';
  const navPath =
    config.navPath ??
    (node.kind === 'Site' || node.kind === 'Route' || node.kind === 'Backend' || node.kind === 'Upstream'
      ? sitesPath
      : undefined);

  return (
    <div className="w-60 space-y-3 rounded-xl border border-border bg-surface p-4 shadow-xl">
      <div className="flex items-start justify-between">
        <div className="flex items-center gap-2">
          <div className={cn('rounded-md p-1.5', config.bg)}>
            <Icon size={14} className={config.color} />
          </div>
          <span className={cn('text-[10px] font-bold uppercase tracking-wide', config.color)}>
            {node.kind}
          </span>
        </div>
        <button
          type="button"
          onClick={onClose}
          className="rounded p-1 text-text-secondary transition-colors hover:bg-hover"
          aria-label="Close"
        >
          <X size={14} />
        </button>
      </div>

      <div>
        <div className="mb-0.5 text-[10px] text-text-secondary">Name</div>
        <div className="break-all text-[12px] font-semibold text-text">{node.name}</div>
      </div>

      {node.detail ? (
        <div>
          <div className="mb-0.5 text-[10px] text-text-secondary">Detail</div>
          <div className="text-[12px] text-text">{node.detail}</div>
        </div>
      ) : null}

      <div>
        <div className="mb-0.5 text-[10px] text-text-secondary">Status</div>
        <div className="flex items-center gap-1.5">
          <div className={cn('h-2 w-2 shrink-0 rounded-full', dotColor)} />
          <span className="text-[12px] capitalize text-text">{node.status}</span>
        </div>
      </div>

      {navPath ? (
        <button
          type="button"
          onClick={() => navigate(navPath)}
          className="w-full rounded-lg border border-border px-3 py-1.5 text-left text-[11px] text-text-secondary transition-colors hover:bg-hover"
        >
          Open related page →
        </button>
      ) : null}
    </div>
  );
}

export function RouteMap() {
  const mode = useMode();
  const sitesPath = mode === 'ingress' ? '/sites/ingress' : '/sites';
  const containerRef = useRef<HTMLDivElement | null>(null);
  const reactFlowRef = useRef<ReactFlowInstance<Node<FlowNodeData>, Edge> | null>(null);

  const [data, setData] = useState<RouteMapData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [isExporting, setIsExporting] = useState(false);
  const [nodes, setNodes, onNodesChange] = useNodesState<Node<FlowNodeData>>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);
  const [selectedNode, setSelectedNode] = useState<RouteMapNode | null>(null);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [collapsedNodeIds, setCollapsedNodeIds] = useState<Set<string>>(new Set());
  const [isSummaryPanelCollapsed, setIsSummaryPanelCollapsed] = useState(true);

  const fetchMap = useCallback(async () => {
    try {
      const [routesRes, config] = await Promise.all([
        api.routes(),
        api.config().catch(() => null),
      ]);
      setData(buildRouteMapData(routesRes.routes ?? [], config));
      setError('');
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load route map');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchMap();
    const timer = setInterval(fetchMap, REFRESH_INTERVAL);
    return () => clearInterval(timer);
  }, [fetchMap]);

  const toggleNodeCollapse = useCallback((nodeId: string) => {
    setCollapsedNodeIds((previous) => {
      const next = new Set(previous);
      if (next.has(nodeId)) next.delete(nodeId);
      else next.add(nodeId);
      return next;
    });
  }, []);

  const { rfNodes, rfEdges, visibleNodeIds } = useMemo(() => {
    if (!data) return { rfNodes: [], rfEdges: [], visibleNodeIds: new Set<string>() };
    const { visibleNodeIds: nextVisibleNodeIds, outgoing } = computeVisibleNodeIds(
      data.nodes,
      data.edges,
      collapsedNodeIds,
    );
    const visibleApiNodes = data.nodes.filter((node) => nextVisibleNodeIds.has(node.id));
    const visibleApiEdges = data.edges.filter(
      (edge) => nextVisibleNodeIds.has(edge.source) && nextVisibleNodeIds.has(edge.target),
    );
    const { rfNodes: nextRfNodes, rfEdges: nextRfEdges } = computeLayout(
      visibleApiNodes,
      visibleApiEdges,
      (node) => ({
        ...node,
        collapsed: collapsedNodeIds.has(node.id),
        canToggleCollapse: (outgoing.get(node.id)?.length ?? 0) > 0,
        onToggleCollapse: toggleNodeCollapse,
      }),
    );
    return { rfNodes: nextRfNodes, rfEdges: nextRfEdges, visibleNodeIds: nextVisibleNodeIds };
  }, [collapsedNodeIds, data, toggleNodeCollapse]);

  useEffect(() => {
    if (!data) {
      setCollapsedNodeIds(new Set());
      return;
    }
    const validIds = new Set(data.nodes.map((node) => node.id));
    setCollapsedNodeIds((previous) => {
      let changed = false;
      const next = new Set<string>();
      previous.forEach((nodeId) => {
        if (validIds.has(nodeId)) next.add(nodeId);
        else changed = true;
      });
      return changed ? next : previous;
    });
  }, [data]);

  useEffect(() => {
    if (selectedNode && !visibleNodeIds.has(selectedNode.id)) setSelectedNode(null);
  }, [selectedNode, visibleNodeIds]);

  useEffect(() => {
    setNodes(rfNodes);
    setEdges(rfEdges);
  }, [rfNodes, rfEdges, setNodes, setEdges]);

  useEffect(() => {
    const onFullscreenChange = () => {
      setIsFullscreen(document.fullscreenElement === containerRef.current);
    };
    document.addEventListener('fullscreenchange', onFullscreenChange);
    return () => document.removeEventListener('fullscreenchange', onFullscreenChange);
  }, []);

  const onNodeClick = useCallback((_: React.MouseEvent, node: Node) => {
    setSelectedNode(node.data as unknown as RouteMapNode);
  }, []);

  const handleRearrange = useCallback(() => {
    setSelectedNode(null);
    setNodes(
      rfNodes.map((node) => ({
        ...node,
        position: { ...node.position },
        data: { ...node.data },
      })),
    );
    setEdges(
      rfEdges.map((edge) => ({
        ...edge,
        style: edge.style ? { ...edge.style } : edge.style,
        markerEnd: edge.markerEnd,
      })),
    );
    requestAnimationFrame(() => {
      reactFlowRef.current?.fitView({ padding: 0.1, duration: 400 });
    });
  }, [rfEdges, rfNodes, setEdges, setNodes]);

  const handleExportImage = useCallback(async () => {
    if (!reactFlowRef.current || rfNodes.length === 0) {
      toast.error('No route map available to export.');
      return;
    }
    const viewportEl = containerRef.current?.querySelector('.react-flow__renderer') as HTMLElement | null;
    if (!viewportEl) {
      toast.error('Unable to locate route map viewport.');
      return;
    }
    setIsExporting(true);
    try {
      const dataUrl = await toPng(viewportEl, {
        cacheBust: true,
        pixelRatio: 2,
        backgroundColor:
          getComputedStyle(document.documentElement).getPropertyValue('--color-bg').trim() || '#0b1020',
      });
      const stamp = new Date().toISOString().replace(/[:.]/g, '-');
      const link = document.createElement('a');
      link.download = `route-map-${stamp}.png`;
      link.href = dataUrl;
      link.click();
      toast.success('Route map image exported.');
    } catch (exportError) {
      toast.error(exportError instanceof Error ? exportError.message : 'Failed to export route map image.');
    } finally {
      setIsExporting(false);
      requestAnimationFrame(() => {
        reactFlowRef.current?.fitView({ padding: 0.1, duration: 0 });
      });
    }
  }, [rfNodes]);

  const toggleFullscreen = useCallback(async () => {
    const el = containerRef.current;
    if (!el) return;
    try {
      if (document.fullscreenElement === el) {
        await document.exitFullscreen();
        return;
      }
      if (document.fullscreenElement) await document.exitFullscreen();
      if ('requestFullscreen' in el) {
        await el.requestFullscreen();
        return;
      }
      const safariEl = el as HTMLElement & { webkitRequestFullscreen?: () => Promise<void> | void };
      if (typeof safariEl.webkitRequestFullscreen === 'function') {
        await safariEl.webkitRequestFullscreen();
      }
    } catch {
      // Ignore fullscreen errors.
    }
  }, []);

  const stats = useMemo(() => {
    if (!data) return [];
    const map: Record<string, number> = {};
    data.nodes.forEach((n) => {
      map[n.kind] = (map[n.kind] ?? 0) + 1;
    });
    const order = Object.keys(KIND_CONFIG);
    return Object.entries(map).sort((a, b) => order.indexOf(a[0]) - order.indexOf(b[0]));
  }, [data]);

  if (loading) {
    return (
      <div className="flex h-64 items-center justify-center">
        <div className="flex flex-col items-center gap-2">
          <Loader size={24} className="animate-spin text-primary" />
          <p className="text-sm text-text-secondary">Loading route map…</p>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex h-64 flex-col items-center justify-center gap-3">
        <p className="text-sm text-red-r1">{error}</p>
        <button
          type="button"
          onClick={() => {
            setLoading(true);
            fetchMap();
          }}
          className="rounded-lg border border-border px-3 py-1.5 text-xs hover:bg-hover"
        >
          Retry
        </button>
      </div>
    );
  }

  if (!data || data.nodes.length === 0) {
    return (
      <div className="flex h-64 flex-col items-center justify-center gap-2 text-text-secondary">
        <MapIcon size={28} className="opacity-50" />
        <p className="text-sm font-medium">No routes found</p>
        <p className="max-w-xs text-center text-xs">
          Add sites and paths under Sites, then the live router graph will appear here.
        </p>
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      className={cn('relative', isFullscreen ? 'm-0' : '-m-4')}
      style={{ height: isFullscreen ? '100vh' : 'calc(100vh - 64px)' }}
    >
      <style>{`
        .route-map-controls {
          box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
          overflow: hidden;
        }
        .route-map-controls .react-flow__controls-button {
          background: var(--color-surface);
          border-bottom: 1px solid var(--color-border);
          color: var(--color-text-secondary);
        }
        .route-map-controls .react-flow__controls-button:last-child {
          border-bottom: 0;
        }
        .route-map-controls .react-flow__controls-button:hover {
          background: var(--color-hover);
          color: var(--color-text);
        }
        .route-map-controls .react-flow__controls-button svg {
          fill: currentColor;
        }
      `}</style>
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onInit={(instance) => {
          reactFlowRef.current = instance;
        }}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        nodeTypes={nodeTypes}
        onNodeClick={onNodeClick}
        onPaneClick={() => setSelectedNode(null)}
        fitView
        fitViewOptions={{ padding: 0.1 }}
        minZoom={0.05}
        maxZoom={3}
        style={{ background: 'transparent' }}
        proOptions={{ hideAttribution: true }}
      >
        <Background variant={BackgroundVariant.Dots} gap={24} size={1} color="var(--color-border)" />
        <Controls
          className="route-map-controls"
          position="bottom-right"
          style={{
            background: 'var(--color-surface)',
            border: '1px solid var(--color-border)',
            borderRadius: 8,
          }}
        />
        <MiniMap
          position="bottom-left"
          style={{
            background: 'var(--color-surface)',
            border: '1px solid var(--color-border)',
            borderRadius: 8,
          }}
          nodeColor={(node) => MINIMAP_COLOR[(node.data as unknown as RouteMapNode)?.kind] ?? '#6b7280'}
          maskColor="rgba(0,0,0,0.15)"
        />

        <Panel position="top-left">
          <div className="flex flex-col gap-2" style={{ maxWidth: 480 }}>
            <div className="rounded-lg border border-border bg-surface shadow-sm">
              <button
                type="button"
                onClick={() => setIsSummaryPanelCollapsed((value) => !value)}
                className="flex w-full items-center justify-between gap-2 px-3 py-2 text-left text-[11px] font-semibold text-text transition-colors hover:bg-hover"
                aria-expanded={!isSummaryPanelCollapsed}
              >
                <span>Map summary</span>
                {isSummaryPanelCollapsed ? <ChevronDown size={14} /> : <ChevronUp size={14} />}
              </button>

              {!isSummaryPanelCollapsed ? (
                <div className="flex flex-col gap-2 border-t border-border p-2">
                  <div className="flex flex-wrap gap-1.5">
                    {stats.map(([kind, count]) => {
                      const cfg = KIND_CONFIG[kind];
                      if (!cfg) return null;
                      const Icon = cfg.icon;
                      return (
                        <div
                          key={kind}
                          className={cn(
                            'flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-semibold',
                            cfg.bg,
                            cfg.color,
                          )}
                        >
                          <Icon size={10} />
                          <span>
                            {kind}: {count}
                          </span>
                        </div>
                      );
                    })}
                  </div>

                  <div className="space-y-1 rounded-lg border border-border bg-surface p-2 shadow-sm">
                    {(
                      [
                        { color: '#ef4444', dash: '4 3', label: 'covers (Certificate → Site)' },
                        { color: '#f97316', dash: '5 3', label: 'routes (Site → Route)' },
                        { color: '#22c55e', dash: '4 2', label: 'uses (Route → Backend)' },
                        { color: 'var(--color-border)', label: 'owns (Backend → Upstream)' },
                        { color: '#3b82f6', label: 'forwards (Route → Upstream override)' },
                      ] as Array<{ color: string; dash?: string; label: string }>
                    ).map(({ color, dash, label }) => (
                      <div key={label} className="flex items-center gap-2 text-[10px] text-text-secondary">
                        <svg width="28" height="8" className="shrink-0">
                          <line
                            x1="0"
                            y1="4"
                            x2="28"
                            y2="4"
                            stroke={color}
                            strokeWidth="1.5"
                            strokeDasharray={dash}
                          />
                          <polygon points="22,1 28,4 22,7" fill={color} />
                        </svg>
                        <span>{label}</span>
                      </div>
                    ))}
                  </div>
                </div>
              ) : null}
            </div>
          </div>
        </Panel>

        <Panel position="top-right">
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => {
                setLoading(true);
                void fetchMap();
              }}
              className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-surface px-2 py-1.5 text-[11px] font-medium text-text-secondary shadow-sm transition-colors hover:bg-hover hover:text-text"
              title="Refresh from live router"
            >
              <RefreshCw size={12} />
              <span>Refresh</span>
            </button>
            <button
              type="button"
              onClick={handleRearrange}
              className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-surface px-2 py-1.5 text-[11px] font-medium text-text-secondary shadow-sm transition-colors hover:bg-hover hover:text-text"
              title="Rearrange nodes"
            >
              <MapIcon size={12} />
              <span>Rearrange</span>
            </button>
            <button
              type="button"
              onClick={handleExportImage}
              disabled={isExporting}
              className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-surface px-2 py-1.5 text-[11px] font-medium text-text-secondary shadow-sm transition-colors hover:bg-hover hover:text-text disabled:cursor-not-allowed disabled:opacity-60"
              title="Export map as PNG"
            >
              <Upload size={12} />
              <span>{isExporting ? 'Exporting…' : 'Export image'}</span>
            </button>
            <button
              type="button"
              onClick={toggleFullscreen}
              className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-surface px-2 py-1.5 text-[11px] font-medium text-text-secondary shadow-sm transition-colors hover:bg-hover hover:text-text"
            >
              {isFullscreen ? <Minimize2 size={12} /> : <Maximize2 size={12} />}
              <span>{isFullscreen ? 'Exit fullscreen' : 'Fullscreen'}</span>
            </button>
          </div>
        </Panel>
      </ReactFlow>

      {selectedNode ? (
        <div className="absolute right-4 top-4 z-10">
          <DetailDrawer node={selectedNode} sitesPath={sitesPath} onClose={() => setSelectedNode(null)} />
        </div>
      ) : null}
    </div>
  );
}
