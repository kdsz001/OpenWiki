import { BookOpen, User, FileText, GitCompare, Layers, MessageCircle, Briefcase, Network, LayoutDashboard } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { WikiPage } from "../../types/wiki";

const TYPE_CONFIG_BASE: Record<string, { icon: React.ComponentType<{ className?: string; size?: number; style?: React.CSSProperties }>; labelKey: string; color: string }> = {
  concept: { icon: BookOpen, labelKey: "browse.pageType.concept", color: "#F97316" },
  entity: { icon: User, labelKey: "browse.pageType.entity", color: "#2563EB" },
  source: { icon: FileText, labelKey: "browse.pageType.source", color: "#16A34A" },
  case: { icon: Briefcase, labelKey: "browse.pageType.case", color: "#DC2626" },
  theme: { icon: Network, labelKey: "browse.pageType.theme", color: "#0F766E" },
  comparison: { icon: GitCompare, labelKey: "browse.pageType.comparison", color: "#CA8A04" },
  dashboard: { icon: LayoutDashboard, labelKey: "browse.pageType.dashboard", color: "#7C3AED" },
  overview: { icon: Layers, labelKey: "browse.pageType.overview", color: "#7C3AED" },
  qa: { icon: MessageCircle, labelKey: "card.qa", color: "#78716C" },
};

interface WikiPageCardProps {
  page: WikiPage;
  onClick: () => void;
}

export function WikiPageCard({ page, onClick }: WikiPageCardProps) {
  const { t } = useTranslation("wiki");

  function timeAgo(dateStr: string): string {
    const now = new Date();
    const then = new Date(dateStr);
    const diffMs = now.getTime() - then.getTime();
    const hours = Math.floor(diffMs / (1000 * 60 * 60));
    if (hours < 1) return t("card.timeAgo.justNow");
    if (hours < 24) return t("card.timeAgo.hoursAgo", { count: hours });
    const days = Math.floor(hours / 24);
    if (days === 1) return t("card.timeAgo.yesterday");
    if (days < 30) return t("card.timeAgo.daysAgo", { count: days });
    return t("card.timeAgo.monthsAgo", { count: Math.floor(days / 30) });
  }

  const config = TYPE_CONFIG_BASE[page.page_type] || TYPE_CONFIG_BASE.concept;
  const IconComponent = config.icon;
  const tags: string[] = page.tags ? JSON.parse(page.tags) : [];
  const isStale = page.status === "needs_recompile";

  return (
    <button
      onClick={onClick}
      className="w-full text-left rounded-xl p-4 transition-all duration-150 hover:scale-[1.01] active:scale-[0.99]"
      style={{
        backgroundColor: "var(--color-surface, #FFFFFF)",
        border: `1px solid ${isStale ? "#CA8A0440" : "var(--color-border, #E7E5E4)"}`,
      }}
    >
      {/* Header: type badge + time */}
      <div className="flex items-center justify-between mb-2">
        <div className="flex items-center gap-1.5">
          <IconComponent size={14} style={{ color: config.color }} />
          <span
            className="text-[10px] font-semibold px-1.5 py-0.5 rounded"
            style={{ color: config.color, backgroundColor: `${config.color}15` }}
          >
            {t(config.labelKey)}
          </span>
          {isStale && (
            <span className="text-[10px] font-medium px-1.5 py-0.5 rounded bg-amber-50 dark:bg-amber-500/10 text-amber-600 dark:text-amber-400">
              ⚠ {t("card.stale")}
            </span>
          )}
        </div>
        <span style={{ fontSize: 11, color: "var(--color-text-muted, #A8A29E)" }}>
          {timeAgo(page.updated_at)}
        </span>
      </div>

      {/* Title */}
      <h3
        className="font-semibold mb-1 line-clamp-2"
        style={{ fontSize: 15, color: "var(--color-text-primary, #1C1917)" }}
      >
        {page.title}
      </h3>

      {/* Summary */}
      {page.summary && (
        <p
          className="line-clamp-2 mb-2"
          style={{ fontSize: 13, lineHeight: 1.6, color: "var(--color-text-secondary, #57534E)" }}
        >
          {page.summary}
        </p>
      )}

      {/* Tags */}
      {tags.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {tags.slice(0, 4).map((tag, i) => (
            <span
              key={i}
              className="rounded-full px-2 py-0.5"
              style={{
                fontSize: 11,
                color: "#F97316",
                backgroundColor: "#F9731610",
                border: "1px solid #F9731625",
              }}
            >
              {tag}
            </span>
          ))}
        </div>
      )}

      {/* Confidence bar (only show if not 1.0) */}
      {page.confidence < 0.95 && (
        <div className="mt-2 flex items-center gap-2">
          <div className="flex-1 h-1 rounded-full" style={{ backgroundColor: "var(--color-border, #E7E5E4)" }}>
            <div
              className="h-1 rounded-full transition-all"
              style={{
                width: `${page.confidence * 100}%`,
                backgroundColor: page.confidence >= 0.8 ? "#16A34A" : page.confidence >= 0.5 ? "#CA8A04" : "#DC2626",
              }}
            />
          </div>
          <span style={{ fontSize: 10, color: "var(--color-text-muted, #A8A29E)" }}>
            {Math.round(page.confidence * 100)}%
          </span>
        </div>
      )}
    </button>
  );
}
