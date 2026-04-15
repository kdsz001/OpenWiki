import { useEffect, useRef, useCallback, Component, type ReactNode } from "react";
import { forceSimulation, forceLink, forceManyBody, forceCenter, forceX, forceY, type SimulationNodeDatum, type SimulationLinkDatum } from "d3-force";
import { useTranslation } from "react-i18next";
import { useWikiStore } from "../../stores/wikiStore";
import { WikiPageDetail } from "./WikiPageDetail";

class GraphErrorBoundary extends Component<{ children: ReactNode; errorText: string; retryText: string }, { error: string | null }> {
  state = { error: null as string | null };
  static getDerivedStateFromError(e: Error) { return { error: e.message }; }
  render() {
    if (this.state.error) {
      return (
        <div className="flex flex-col items-center justify-center py-16 gap-2">
          <p style={{ fontSize: 14, fontWeight: 600, color: "var(--color-text-primary)" }}>{this.props.errorText}</p>
          <p style={{ fontSize: 12, color: "var(--color-text-muted)" }}>{this.state.error}</p>
          <button onClick={() => this.setState({ error: null })}
            className="mt-2 px-3 py-1.5 rounded-lg text-xs font-medium"
            style={{ color: "#F97316", backgroundColor: "#F9731615", border: "1px solid #F9731630" }}>
            {this.props.retryText}
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

const TYPE_COLORS: Record<string, string> = {
  concept: "#F97316",
  entity: "#2563EB",
  source: "#16A34A",
  case: "#DC2626",
  theme: "#0F766E",
  comparison: "#CA8A04",
  dashboard: "#7C3AED",
  overview: "#7C3AED",
};

const TYPE_LABEL_KEYS: Record<string, string> = {
  concept: "browse.pageType.concept",
  entity: "browse.pageType.entity",
  source: "browse.pageType.source",
  case: "browse.pageType.case",
  theme: "browse.pageType.theme",
  comparison: "browse.pageType.comparison",
  dashboard: "browse.pageType.dashboard",
  overview: "browse.pageType.overview",
};

interface GNode extends SimulationNodeDatum {
  id: string;
  title: string;
  page_type: string;
  status: string;
  confidence: number;
  edge_count: number;
}

interface GLink extends SimulationLinkDatum<GNode> {
  relation: string;
  weight: number;
}

function WikiGraphViewInner() {
  const { t } = useTranslation("wiki");
  const { graphData, isLoadingGraph, loadGraph, selectedPage, selectPage, clearSelection, deletePage } = useWikiStore();
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const simRef = useRef<ReturnType<typeof forceSimulation<GNode, GLink>> | null>(null);
  const rafRef = useRef<number>(0);
  const nodesRef = useRef<GNode[]>([]);
  const linksRef = useRef<GLink[]>([]);
  const camRef = useRef({ x: 0, y: 0, zoom: 1 });
  const dragRef = useRef<{ node: GNode | null; startX: number; startY: number; moved: boolean }>({ node: null, startX: 0, startY: 0, moved: false });
  const panRef = useRef<{ active: boolean; lastX: number; lastY: number }>({ active: false, lastX: 0, lastY: 0 });
  const hoverRef = useRef<GNode | null>(null);

  useEffect(() => { loadGraph(); }, [loadGraph]);

  const getNodeRadius = useCallback((node: GNode) => {
    return Math.max(4, 3 + Math.sqrt(node.edge_count || 0) * 2);
  }, []);

  const screenToGraph = useCallback((sx: number, sy: number, canvas: HTMLCanvasElement) => {
    const rect = canvas.getBoundingClientRect();
    const cam = camRef.current;
    return {
      x: (sx - rect.left - rect.width / 2) / cam.zoom - cam.x,
      y: (sy - rect.top - rect.height / 2) / cam.zoom - cam.y,
    };
  }, []);

  const findNode = useCallback((gx: number, gy: number) => {
    const nodes = nodesRef.current;
    for (let i = nodes.length - 1; i >= 0; i--) {
      const n = nodes[i];
      const r = getNodeRadius(n) + 4;
      const dx = (n.x || 0) - gx, dy = (n.y || 0) - gy;
      if (dx * dx + dy * dy < r * r) return n;
    }
    return null;
  }, [getNodeRadius]);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const dpr = window.devicePixelRatio || 1;
    const w = canvas.width / dpr;
    const h = canvas.height / dpr;
    const cam = camRef.current;
    const isDark = document.documentElement.classList.contains("dark");

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    ctx.fillStyle = isDark ? "#1C1917" : "#F5F5F0";
    ctx.fillRect(0, 0, w, h);

    ctx.save();
    ctx.translate(w / 2, h / 2);
    ctx.scale(cam.zoom, cam.zoom);
    ctx.translate(cam.x, cam.y);

    // Links
    ctx.strokeStyle = isDark ? "rgba(168,162,158,0.25)" : "rgba(120,113,108,0.3)";
    ctx.lineWidth = 0.5 / cam.zoom;
    for (const link of linksRef.current) {
      const s = link.source as GNode, t = link.target as GNode;
      if (s.x == null || t.x == null) continue;
      ctx.beginPath();
      ctx.moveTo(s.x, s.y!);
      ctx.lineTo(t.x, t.y!);
      ctx.stroke();
    }

    // Nodes
    for (const node of nodesRef.current) {
      if (node.x == null) continue;
      const r = getNodeRadius(node);
      ctx.beginPath();
      ctx.arc(node.x, node.y!, r, 0, Math.PI * 2);
      ctx.fillStyle = TYPE_COLORS[node.page_type] || "#A8A29E";
      ctx.fill();
    }

    // Labels when zoomed
    if (cam.zoom > 1.5) {
      const fontSize = Math.min(10 / cam.zoom, 10);
      ctx.font = `${fontSize}px sans-serif`;
      ctx.textAlign = "center";
      ctx.textBaseline = "top";
      ctx.fillStyle = isDark ? "rgba(250,250,248,0.7)" : "rgba(28,25,23,0.7)";
      for (const node of nodesRef.current) {
        if (node.x == null) continue;
        const r = getNodeRadius(node);
        const label = node.title.length > 10 ? node.title.slice(0, 10) + "…" : node.title;
        ctx.fillText(label, node.x, node.y! + r + 2);
      }
    }

    // Hover highlight
    const hovered = hoverRef.current;
    if (hovered && hovered.x != null) {
      const r = getNodeRadius(hovered);
      ctx.beginPath();
      ctx.arc(hovered.x, hovered.y!, r + 2, 0, Math.PI * 2);
      ctx.strokeStyle = "#F97316";
      ctx.lineWidth = 2 / cam.zoom;
      ctx.stroke();
      ctx.font = `bold ${12 / cam.zoom}px sans-serif`;
      ctx.fillStyle = isDark ? "#FAFAF8" : "#1C1917";
      ctx.textAlign = "center";
      ctx.textBaseline = "bottom";
      ctx.fillText(hovered.title, hovered.x, hovered.y! - r - 4);
    }

    ctx.restore();
  }, [getNodeRadius]);

  const tick = useCallback(() => {
    draw();
    rafRef.current = requestAnimationFrame(tick);
  }, [draw]);

  // Setup simulation
  useEffect(() => {
    if (!graphData || graphData.nodes.length === 0) return;

    const nodes: GNode[] = graphData.nodes.map(n => ({ ...n }));
    const nodeIds = new Set(nodes.map(n => n.id));
    const links: GLink[] = graphData.edges
      .filter(e => nodeIds.has(e.source) && nodeIds.has(e.target))
      .map(e => ({ source: e.source, target: e.target, relation: e.relation, weight: e.weight }));

    nodesRef.current = nodes;
    linksRef.current = links as any;

    simRef.current?.stop();

    const sim = forceSimulation<GNode, GLink>(nodes)
      .force("charge", forceManyBody<GNode>().strength(-80).distanceMax(300))
      .force("link", forceLink<GNode, GLink>(links).id(d => d.id).distance(50))
      .force("center", forceCenter(0, 0).strength(0.05))
      .force("x", forceX<GNode>(0).strength(0.02))
      .force("y", forceY<GNode>(0).strength(0.02))
      .alphaDecay(0.028)
      .velocityDecay(0.4)
      .alpha(1);

    simRef.current = sim;
    cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(tick);

    return () => { sim.stop(); cancelAnimationFrame(rafRef.current); };
  }, [graphData, tick]);

  // Resize canvas
  useEffect(() => {
    const container = containerRef.current;
    const canvas = canvasRef.current;
    if (!container || !canvas) return;
    const dpr = window.devicePixelRatio || 1;
    const resize = () => {
      const w = container.clientWidth, h = container.clientHeight;
      canvas.width = w * dpr;
      canvas.height = h * dpr;
      canvas.style.width = w + "px";
      canvas.style.height = h + "px";
    };
    resize();
    const obs = new ResizeObserver(resize);
    obs.observe(container);
    return () => obs.disconnect();
  }, []);

  // Pointer events
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const onDown = (e: PointerEvent) => {
      const g = screenToGraph(e.clientX, e.clientY, canvas);
      const node = findNode(g.x, g.y);
      if (node) {
        dragRef.current = { node, startX: e.clientX, startY: e.clientY, moved: false };
        node.fx = node.x;
        node.fy = node.y;
        simRef.current?.alphaTarget(0.3).restart();
        canvas.setPointerCapture(e.pointerId);
      } else {
        panRef.current = { active: true, lastX: e.clientX, lastY: e.clientY };
      }
    };

    const onMove = (e: PointerEvent) => {
      const drag = dragRef.current;
      if (drag.node) {
        if (Math.abs(e.clientX - drag.startX) > 3 || Math.abs(e.clientY - drag.startY) > 3) drag.moved = true;
        const g = screenToGraph(e.clientX, e.clientY, canvas);
        drag.node.fx = g.x;
        drag.node.fy = g.y;
      } else if (panRef.current.active) {
        const cam = camRef.current;
        cam.x += (e.clientX - panRef.current.lastX) / cam.zoom;
        cam.y += (e.clientY - panRef.current.lastY) / cam.zoom;
        panRef.current.lastX = e.clientX;
        panRef.current.lastY = e.clientY;
      } else {
        const g = screenToGraph(e.clientX, e.clientY, canvas);
        hoverRef.current = findNode(g.x, g.y);
        canvas.style.cursor = hoverRef.current ? "pointer" : "grab";
      }
    };

    const onUp = (e: PointerEvent) => {
      const drag = dragRef.current;
      if (drag.node) {
        if (!drag.moved) selectPage(drag.node.id);
        drag.node.fx = null;
        drag.node.fy = null;
        simRef.current?.alphaTarget(0);
        drag.node = null;
        canvas.releasePointerCapture(e.pointerId);
      }
      panRef.current.active = false;
    };

    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      camRef.current.zoom = Math.max(0.1, Math.min(10, camRef.current.zoom * (e.deltaY > 0 ? 0.97 : 1.03)));
    };

    canvas.addEventListener("pointerdown", onDown);
    canvas.addEventListener("pointermove", onMove);
    canvas.addEventListener("pointerup", onUp);
    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      canvas.removeEventListener("pointerdown", onDown);
      canvas.removeEventListener("pointermove", onMove);
      canvas.removeEventListener("pointerup", onUp);
      canvas.removeEventListener("wheel", onWheel);
    };
  }, [screenToGraph, findNode, selectPage]);

  const isDark = document.documentElement.classList.contains("dark");

  return (
    <div ref={containerRef} className="relative rounded-xl overflow-hidden" style={{
      height: "calc(100vh - 170px)",
      backgroundColor: isDark ? "#1C1917" : "#F5F5F0",
      border: "1px solid var(--color-border, #E7E5E4)",
      marginTop: 8,
    }}>
      <div className="absolute top-3 left-3 z-10 flex gap-3" style={{ pointerEvents: "none" }}>
        {Object.entries(TYPE_COLORS).map(([type, color]) => (
          <div key={type} className="flex items-center gap-1">
            <span className="w-2 h-2 rounded-full" style={{ backgroundColor: color }} />
            <span style={{ fontSize: 10, color: "var(--color-text-muted)" }}>
              {t(TYPE_LABEL_KEYS[type])}
            </span>
          </div>
        ))}
      </div>

      <canvas ref={canvasRef} style={{ display: "block", width: "100%", height: "100%" }} />

      {isLoadingGraph && (
        <div className="absolute inset-0 flex items-center justify-center" style={{ pointerEvents: "none" }}>
          <div className="w-6 h-6 border-2 border-orange-500 border-t-transparent rounded-full animate-spin" />
        </div>
      )}

      {selectedPage && (
        <WikiPageDetail
          page={selectedPage}
          onClose={clearSelection}
          onDelete={(id) => { deletePage(id); clearSelection(); loadGraph(); }}
          onNavigateToPage={(pageId) => selectPage(pageId)}
        />
      )}
    </div>
  );
}

const _WikiGraphViewInner = WikiGraphViewInner;
export function WikiGraphView() {
  const { t } = useTranslation("wiki");
  return (
    <GraphErrorBoundary errorText={t("graph.error")} retryText={t("graph.retry")}>
      <_WikiGraphViewInner />
    </GraphErrorBoundary>
  );
}
