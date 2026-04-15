import { useState, useEffect, useCallback } from "react";
import { X, BookOpen, User, FileText, GitCompare, Layers, Trash2, Briefcase, Network, LayoutDashboard } from "lucide-react";
import { useTranslation } from "react-i18next";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { WikiPage, WikiPageSource } from "../../types/wiki";
import type { CapturedContent } from "../../types/content";
import { getPageSources, getPageThemes, resolveWikiLink } from "../../services/wikiService";
import { invoke } from "@tauri-apps/api/core";

const TYPE_ICONS: Record<string, React.ComponentType<{ className?: string; size?: number; style?: React.CSSProperties }>> = {
  concept: BookOpen,
  entity: User,
  source: FileText,
  case: Briefcase,
  theme: Network,
  comparison: GitCompare,
  dashboard: LayoutDashboard,
  overview: Layers,
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

const SOURCE_STATUS_ICON: Record<string, string> = {
  active: "✓",
  stale: "⚠",
  deleted: "✗",
};

const SOURCE_STATUS_COLOR: Record<string, string> = {
  active: "#16A34A",
  stale: "#CA8A04",
  deleted: "#DC2626",
};

interface WikiPageDetailProps {
  page: WikiPage;
  onClose: () => void;
  onDelete: (id: string) => void;
  onNavigateToContent?: (contentId: string) => void;
  onNavigateToPage?: (pageId: string) => void;
}

function decodeOpenWikiTarget(href: string): string | null {
  const prefix = "openwiki://page/";
  if (!href.startsWith(prefix)) return null;
  const encoded = href.slice(prefix.length);
  if (!encoded) return null;
  try {
    const normalized = encoded.replace(/-/g, "+").replace(/_/g, "/");
    const padded = normalized + "=".repeat((4 - (normalized.length % 4 || 4)) % 4);
    const binary = window.atob(padded);
    const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
    return new TextDecoder().decode(bytes);
  } catch (error) {
    console.error("Failed to decode OpenWiki target:", error);
    return null;
  }
}

export function WikiPageDetail({ page, onClose, onDelete, onNavigateToContent, onNavigateToPage }: WikiPageDetailProps) {
  const { t } = useTranslation("wiki");
  const [sources, setSources] = useState<(WikiPageSource & { content?: CapturedContent })[]>([]);
  const [loadingSources, setLoadingSources] = useState(true);
  const [themes, setThemes] = useState<WikiPage[]>([]);
  const [deleteConfirm, setDeleteConfirm] = useState(false);
  const IconComponent = TYPE_ICONS[page.page_type] || BookOpen;
  const isReadOnlyLocalPage = page.source_message_id?.startsWith("local-wiki:") ?? false;

  useEffect(() => {
    loadSources();
    loadThemes();
  }, [page.id]);

  async function loadSources() {
    setLoadingSources(true);
    try {
      const pageSources = await getPageSources(page.id);
      // Fetch content details for each source
      const enriched = await Promise.all(
        pageSources.map(async (src) => {
          try {
            const content = await invoke<CapturedContent | null>("get_contents_by_ids", {
              ids: [src.content_id],
            });
            return { ...src, content: Array.isArray(content) ? content[0] : undefined };
          } catch {
            return { ...src, content: undefined };
          }
        })
      );
      setSources(enriched);
    } catch (e) {
      console.error("Failed to load sources:", e);
    }
    setLoadingSources(false);
  }

  async function loadThemes() {
    try {
      setThemes(await getPageThemes(page.id));
    } catch (e) {
      console.error("Failed to load page themes:", e);
      setThemes([]);
    }
  }

  const handleWikiLinkClick = useCallback(async (href: string) => {
    const target = decodeOpenWikiTarget(href);
    if (!target) return;
    try {
      const resolvedPage = await resolveWikiLink(target);
      if (resolvedPage) {
        onNavigateToPage?.(resolvedPage.id);
      }
    } catch (error) {
      console.error("Failed to resolve wiki link:", error);
    }
  }, [onNavigateToPage]);

  const isStale = page.status === "needs_recompile";

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center pt-10 pb-10">
      {/* Backdrop */}
      <div className="absolute inset-0 bg-black/30 dark:bg-black/50" onClick={onClose} />

      {/* Panel */}
      <div
        className="relative w-full max-w-2xl max-h-[85vh] overflow-y-auto rounded-2xl shadow-2xl"
        style={{
          backgroundColor: "var(--color-surface, #FFFFFF)",
          border: "1px solid var(--color-border, #E7E5E4)",
        }}
      >
        {/* Header */}
        <div className="sticky top-0 z-10 flex items-center justify-between px-6 py-4 border-b"
          style={{ borderColor: "var(--color-border, #E7E5E4)", backgroundColor: "var(--color-surface, #FFFFFF)" }}
        >
          <div className="flex items-center gap-2">
            <IconComponent size={18} style={{ color: "#F97316" }} />
            <span className="text-[11px] font-semibold px-2 py-0.5 rounded"
              style={{ color: "#F97316", backgroundColor: "#F9731615" }}
            >
              {t(TYPE_LABEL_KEYS[page.page_type]) || page.page_type}
            </span>
            {isStale && (
              <span className="text-[11px] font-medium px-2 py-0.5 rounded bg-amber-50 dark:bg-amber-500/10 text-amber-600 dark:text-amber-400">
                {t("detail.staleWarning")}
              </span>
            )}
            {isReadOnlyLocalPage && (
              <span className="text-[11px] font-medium px-2 py-0.5 rounded bg-sky-50 dark:bg-sky-500/10 text-sky-600 dark:text-sky-400">
                {t("detail.readOnly")}
              </span>
            )}
          </div>
          <div className="flex items-center gap-1">
            {!isReadOnlyLocalPage && deleteConfirm ? (
              <div className="flex items-center gap-1">
                <button
                  onClick={() => { onDelete(page.id); setDeleteConfirm(false); }}
                  className="px-2 py-1 rounded-md text-[11px] font-medium text-white bg-red-500 hover:bg-red-600 transition-colors"
                >
                  {t("detail.confirmDelete")}
                </button>
                <button
                  onClick={() => setDeleteConfirm(false)}
                  className="px-2 py-1 rounded-md text-[11px] text-stone-400 hover:text-stone-600 transition-colors"
                >
                  {t("detail.cancel")}
                </button>
              </div>
            ) : !isReadOnlyLocalPage ? (
              <button
                onClick={() => setDeleteConfirm(true)}
                className="p-1.5 rounded-lg hover:bg-red-50 dark:hover:bg-red-500/10 text-stone-400 hover:text-red-500 transition-colors"
                title={t("detail.deleteTooltip")}
              >
                <Trash2 size={16} />
              </button>
            ) : null}
            <button
              onClick={onClose}
              className="p-1.5 rounded-lg hover:bg-stone-100 dark:hover:bg-white/[0.08] text-stone-400 transition-colors"
            >
              <X size={16} />
            </button>
          </div>
        </div>

        {/* Body */}
        <div className="px-6 py-5">
          {/* Title */}
          <h1
            className="font-bold mb-2"
            style={{ fontSize: 22, fontFamily: "'Cabinet Grotesk', sans-serif", color: "var(--color-text-primary, #1C1917)" }}
          >
            {page.title}
          </h1>

          {/* Summary */}
          {page.summary && (
            <p className="mb-4" style={{ fontSize: 14, color: "var(--color-text-secondary, #57534E)" }}>
              {page.summary}
            </p>
          )}
          {isReadOnlyLocalPage && (
            <p className="mb-4 text-xs" style={{ color: "var(--color-text-muted, #78716C)" }}>
              {t("detail.readOnlyDesc")}
            </p>
          )}

          {themes.length > 0 && (
            <div className="mb-5">
              <h3 className="flex items-center gap-1.5 mb-2" style={{ fontSize: 13, fontWeight: 600, color: "var(--color-text-primary)" }}>
                <Network size={14} style={{ color: "#0F766E" }} />
                {t("detail.themes")}
              </h3>
              <div className="flex flex-wrap gap-2">
                {themes.map((theme) => (
                  <button
                    key={theme.id}
                    onClick={() => onNavigateToPage?.(theme.id)}
                    className="rounded-full px-3 py-1 text-xs font-medium transition-colors"
                    style={{
                      color: "#0F766E",
                      backgroundColor: "#0F766E12",
                      border: "1px solid #0F766E30",
                    }}
                  >
                    {theme.title}
                  </button>
                ))}
              </div>
            </div>
          )}

          {/* Markdown content */}
          <article
            className="prose prose-sm prose-stone dark:prose-invert max-w-none mb-6
                       prose-headings:font-bold prose-headings:text-stone-800 dark:prose-headings:text-stone-200
                       prose-p:text-stone-600 dark:prose-p:text-stone-300
                       prose-a:text-orange-500 prose-a:no-underline hover:prose-a:underline
                       prose-strong:text-stone-700 dark:prose-strong:text-stone-200
                       prose-code:text-orange-600 dark:prose-code:text-orange-400
                       prose-code:bg-orange-50 dark:prose-code:bg-orange-500/10
                       prose-code:px-1 prose-code:py-0.5 prose-code:rounded
                       prose-code:before:content-none prose-code:after:content-none"
            style={{ fontSize: 14, lineHeight: 1.8 }}
          >
            <ReactMarkdown
              remarkPlugins={[remarkGfm]}
              urlTransform={(url) => {
                if (url.startsWith("openwiki://page/")) return url;
                return url;
              }}
              components={{
                a: ({ href, children, ...props }) => {
                  if (href?.startsWith("openwiki://page/")) {
                    return (
                      <button
                        type="button"
                        onClick={() => handleWikiLinkClick(href)}
                        className="inline p-0 m-0 border-0 bg-transparent text-orange-500 hover:underline cursor-pointer"
                      >
                        {children}
                      </button>
                    );
                  }
                  return (
                    <a
                      {...props}
                      href={href}
                      target="_blank"
                      rel="noreferrer"
                    >
                      {children}
                    </a>
                  );
                },
              }}
            >
              {page.body_markdown}
            </ReactMarkdown>
          </article>

          {/* Sources section */}
          <div className="border-t pt-4" style={{ borderColor: "var(--color-border, #E7E5E4)" }}>
            <h3 className="flex items-center gap-1.5 mb-3" style={{ fontSize: 13, fontWeight: 600, color: "var(--color-text-primary)" }}>
              <span className="w-1 h-1 rounded-full" style={{ backgroundColor: "#F97316" }} />
              {t("detail.compiledFrom")}
            </h3>

            {loadingSources ? (
              <div className="text-xs" style={{ color: "var(--color-text-muted)" }}>{t("detail.loading")}</div>
            ) : sources.length === 0 ? (
              <div className="text-xs" style={{ color: "var(--color-text-muted)" }}>{t("detail.noSources")}</div>
            ) : (
              <div className="space-y-2">
                {sources.map((src) => (
                  <button
                    key={src.id}
                    onClick={() => src.content && onNavigateToContent?.(src.content_id)}
                    className="w-full text-left flex items-center gap-3 p-3 rounded-lg transition-colors hover:bg-stone-50 dark:hover:bg-white/[0.04]"
                    style={{ border: "1px solid var(--color-border, #E7E5E4)" }}
                  >
                    <span style={{ color: SOURCE_STATUS_COLOR[src.source_status], fontSize: 14, fontWeight: 700 }}>
                      {SOURCE_STATUS_ICON[src.source_status]}
                    </span>
                    <div className="flex-1 min-w-0">
                      <p className="text-xs truncate" style={{ color: "var(--color-text-primary)" }}>
                        {src.content?.raw_text?.slice(0, 80) || src.content?.source_url || t("detail.contentDeleted")}
                      </p>
                      <p className="text-[10px] mt-0.5" style={{ color: "var(--color-text-muted)" }}>
                        {src.content?.source_app || t("detail.unknownApp")} · {src.contributed_at?.slice(0, 10)}
                      </p>
                    </div>
                  </button>
                ))}
              </div>
            )}
          </div>

          {/* Confidence footer */}
          <div className="mt-4 pt-3 flex items-center justify-between border-t" style={{ borderColor: "var(--color-border)" }}>
            <div className="flex items-center gap-2">
              <span style={{ fontSize: 11, color: "var(--color-text-muted)" }}>{t("detail.confidence")}</span>
              <div className="w-20 h-1.5 rounded-full" style={{ backgroundColor: "var(--color-border)" }}>
                <div
                  className="h-1.5 rounded-full"
                  style={{
                    width: `${page.confidence * 100}%`,
                    backgroundColor: page.confidence >= 0.8 ? "#16A34A" : page.confidence >= 0.5 ? "#CA8A04" : "#DC2626",
                  }}
                />
              </div>
              <span style={{ fontSize: 11, fontFamily: "'JetBrains Mono', monospace", color: "var(--color-text-muted)" }}>
                {Math.round(page.confidence * 100)}%
              </span>
            </div>
            <span style={{ fontSize: 11, color: "var(--color-text-muted)" }}>
              {page.last_compiled_at ? t("detail.compiledAt", { date: page.last_compiled_at.slice(0, 10) }) : t("detail.notCompiled")} · {t("detail.sourceCount", { count: sources.length })}
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
