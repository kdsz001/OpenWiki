import { useState, useEffect } from "react";
import { List, Share2, MessageCircle, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useWikiStore } from "../../stores/wikiStore";
import { WikiBrowseView } from "./WikiBrowseView";
import { WikiGraphView } from "./WikiGraphView";
import { WikiAskSidebar } from "./WikiAskSidebar";

type SubView = "browse" | "graph";

export function WikiView() {
  const { t } = useTranslation("wiki");
  const [subView, setSubView] = useState<SubView>("browse");
  const { stats, loadStats, loadPages, loadGraph, selectedPage, selectPage } = useWikiStore();
  const [askOpen, setAskOpen] = useState(false);
  const [isRefreshing, setIsRefreshing] = useState(false);

  useEffect(() => {
    loadStats();
  }, [loadStats]);

  const handleRefresh = async () => {
    setIsRefreshing(true);
    try {
      if (subView === "graph") {
        await Promise.all([
          loadStats(),
          loadGraph(),
          selectedPage ? selectPage(selectedPage.id) : Promise.resolve(),
        ]);
      } else {
        await Promise.all([
          loadStats(),
          loadPages(),
          selectedPage ? selectPage(selectedPage.id) : Promise.resolve(),
        ]);
      }
    } finally {
      setIsRefreshing(false);
    }
  };

  const handleNavigateToPage = (pageId: string) => {
    selectPage(pageId);
    setSubView("browse");
  };

  return (
    <div className="flex" style={{ height: "calc(100vh - 44px)" }}>
      {/* Main content area */}
      <div className="flex-1 min-w-0 flex flex-col px-5 py-4" style={{ overflow: subView === "graph" ? "hidden" : "auto" }}>
        {/* Header */}
        <div className="flex items-center justify-between mb-4 flex-shrink-0">
          <div>
            <h2
              style={{
                fontSize: 20,
                fontWeight: 700,
                fontFamily: "'Cabinet Grotesk', sans-serif",
                color: "var(--color-text-primary, #1C1917)",
                letterSpacing: "-0.3px",
              }}
            >
              {t("title")}
            </h2>
            {stats && stats.total_pages > 0 && (
              <p style={{ fontSize: 12, color: "var(--color-text-muted, #A8A29E)", marginTop: 2 }}>
                {stats.total_pages} {t("stats.pages")} · {stats.total_edges} {t("stats.edges")} · {stats.total_sources} {t("stats.sources")}
                {stats.needs_recompile > 0 && (
                  <span className="text-amber-500"> · {stats.needs_recompile} {t("stats.needsRecompile")}</span>
                )}
              </p>
            )}
          </div>

          {/* Controls */}
          <div className="flex items-center gap-2">
            <button
              onClick={handleRefresh}
              disabled={isRefreshing}
              title={t("actions.refreshTooltip")}
              className={`flex items-center gap-1 px-3 py-1.5 rounded-lg text-[12px] font-medium transition-all border
                ${isRefreshing
                  ? "text-orange-500 border-orange-200/60 bg-orange-50/70 cursor-wait"
                  : "text-stone-500 border-stone-200/70 hover:text-orange-500 hover:border-orange-200/70 hover:bg-orange-500/10"
                }`}
            >
              <RefreshCw size={13} className={isRefreshing ? "animate-spin" : ""} />
              <span>{isRefreshing ? t("actions.refreshing") : t("actions.refresh")}</span>
            </button>
            <button
              onClick={() => setAskOpen(!askOpen)}
              className={`flex items-center gap-1 px-3 py-1.5 rounded-lg text-[12px] font-medium transition-all
                ${askOpen
                  ? "bg-orange-500 text-white"
                  : "text-stone-400 hover:text-orange-500 hover:bg-orange-500/10"
                }`}
            >
              <MessageCircle size={13} />
              <span>{t("tabs.ask")}</span>
            </button>
            <div className="inline-flex bg-stone-100/60 dark:bg-white/[0.06] rounded-md p-0.5">
              <button
                onClick={() => setSubView("browse")}
                className={`flex items-center gap-1 px-3 py-1 text-[12px] font-medium rounded transition-all duration-200
                  ${subView === "browse"
                    ? "bg-white dark:bg-white/[0.15] text-orange-500 shadow-sm"
                    : "text-stone-400 hover:text-stone-600 dark:hover:text-stone-300"
                  }`}
              >
                <List size={13} />
                <span>{t("tabs.browse")}</span>
              </button>
              <button
                onClick={() => setSubView("graph")}
                className={`flex items-center gap-1 px-3 py-1 text-[12px] font-medium rounded transition-all duration-200
                  ${subView === "graph"
                    ? "bg-white dark:bg-white/[0.15] text-orange-500 shadow-sm"
                    : "text-stone-400 hover:text-stone-600 dark:hover:text-stone-300"
                  }`}
              >
                <Share2 size={13} />
                <span>{t("tabs.graph")}</span>
              </button>
            </div>
          </div>
        </div>

        {/* Sub-view content */}
        {subView === "browse" && <WikiBrowseView />}
        {subView === "graph" && <WikiGraphView />}
      </div>

      {/* Right sidebar — Q&A chat */}
      {askOpen && (
        <div
          className="flex-shrink-0 h-full"
          style={{ width: "min(400px, 80vw)" }}
        >
          <WikiAskSidebar
            onClose={() => setAskOpen(false)}
            onNavigateToPage={handleNavigateToPage}
          />
        </div>
      )}
    </div>
  );
}
