import { useEffect, useCallback } from "react";
import { BookOpen, User, FileText, GitCompare, Layers, Briefcase, Network, LayoutDashboard } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useWikiStore } from "../../stores/wikiStore";
import { WikiPageCard } from "./WikiPageCard";
import { WikiPageDetail } from "./WikiPageDetail";

const TYPE_FILTERS = [
  { id: null, labelKey: "browse.all", icon: null },
  { id: "concept", labelKey: "browse.pageType.concept", icon: BookOpen },
  { id: "entity", labelKey: "browse.pageType.entity", icon: User },
  { id: "source", labelKey: "browse.pageType.source", icon: FileText },
  { id: "case", labelKey: "browse.pageType.case", icon: Briefcase },
  { id: "theme", labelKey: "browse.pageType.theme", icon: Network },
  { id: "comparison", labelKey: "browse.pageType.comparison", icon: GitCompare },
  { id: "dashboard", labelKey: "browse.pageType.dashboard", icon: LayoutDashboard },
  { id: "overview", labelKey: "browse.pageType.overview", icon: Layers },
] as const;

export function WikiBrowseView() {
  const { t } = useTranslation("wiki");
  const {
    pages, selectedPage, isLoadingPages, filterType, error,
    loadPages, selectPage, clearSelection, setFilterType, deletePage,
  } = useWikiStore();

  useEffect(() => {
    loadPages();
  }, [loadPages]);

  const handleNavigateToContent = useCallback((contentId: string) => {
    clearSelection();
    window.dispatchEvent(
      new CustomEvent("navigate-to-content", { detail: { contentIds: [contentId] } })
    );
  }, [clearSelection]);

  const handleNavigateToPage = useCallback((pageId: string) => {
    selectPage(pageId);
  }, [selectPage]);

  return (
    <div className="flex gap-0 h-full">
      {/* Left sidebar: filters */}
      <div className="w-36 flex-shrink-0 pr-3 border-r" style={{ borderColor: "var(--color-border, #E7E5E4)" }}>
        <div className="space-y-0.5">
          {TYPE_FILTERS.map((f) => {
            const isActive = filterType === f.id;
            const Icon = f.icon;
            return (
              <button
                key={f.id ?? "all"}
                onClick={() => setFilterType(f.id ?? null)}
                className="w-full flex items-center gap-2 px-3 py-1.5 rounded-lg text-left transition-colors"
                style={{
                  fontSize: 13,
                  backgroundColor: isActive ? "#FFF7ED" : "transparent",
                  color: isActive ? "#F97316" : "var(--color-text-secondary, #57534E)",
                  fontWeight: isActive ? 600 : 400,
                }}
              >
                {Icon && <Icon size={14} />}
                <span>{t(f.labelKey)}</span>
              </button>
            );
          })}
        </div>
      </div>

      {/* Main area */}
      <div className="flex-1 pl-4">
        {/* Error */}
        {error && (
          <div className="mb-4 p-3 rounded-lg bg-red-50 dark:bg-red-500/10 text-sm text-red-600 dark:text-red-400">
            {error}
          </div>
        )}

        {/* Loading */}
        {isLoadingPages && (
          <div className="flex items-center justify-center py-12">
            <div className="w-6 h-6 border-2 border-orange-500 border-t-transparent rounded-full animate-spin" />
          </div>
        )}

        {/* Empty state */}
        {!isLoadingPages && pages.length === 0 && (
          <div className="text-center py-16">
            <BookOpen size={40} className="mx-auto mb-3" style={{ color: "var(--color-text-muted)" }} />
            <p style={{ fontSize: 15, fontWeight: 600, color: "var(--color-text-primary)" }}>
              {t("browse.emptyTitle")}
            </p>
            <p className="mt-1" style={{ fontSize: 13, color: "var(--color-text-muted)" }}>
              {t("browse.emptyDescription")}
            </p>
          </div>
        )}

        {/* Page grid */}
        {!isLoadingPages && pages.length > 0 && (
          <div className="grid grid-cols-1 gap-3">
            {pages.map((page) => (
              <WikiPageCard
                key={page.id}
                page={page}
                onClick={() => selectPage(page.id)}
              />
            ))}
          </div>
        )}
      </div>

      {/* Page detail overlay */}
      {selectedPage && (
        <WikiPageDetail
          page={selectedPage}
          onClose={clearSelection}
          onDelete={(id) => { deletePage(id); clearSelection(); }}
          onNavigateToContent={handleNavigateToContent}
          onNavigateToPage={handleNavigateToPage}
        />
      )}
    </div>
  );
}
