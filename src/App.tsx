import { useState, useEffect, useRef, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { ClipboardList, Target, Settings, Search, BookOpen, HardDrive } from "lucide-react";
import { ContentList } from "./features/content-list/ContentList";
import { SettingsView } from "./features/settings/SettingsView";
import { DataHubView } from "./features/data-hub/DataHubView";
import { RadarView } from "./features/digest/RadarView";
import { WikiView } from "./features/wiki/WikiView";
import { UpdateBanner } from "./features/update/UpdateBanner";
import { PreAuthModal } from "./features/automation/PreAuthModal";
import { AutomationNotices } from "./features/automation/AutomationNotices";
import { WorkspaceSetupView } from "./features/setup/WorkspaceSetupView";
import { useSettingsStore } from "./stores/settingsStore";
import { useContentStore } from "./stores/contentStore";
import { searchContent } from "./services/dataHubService";
import { searchWiki } from "./services/wikiService";
import type { CapturedContent } from "./types/content";
import type { WikiPage } from "./types/wiki";
// FloatingBubble is now a separate system-level window (see BubbleView.tsx)

type TabId = "content" | "wiki" | "digest" | "datahub" | "settings";

interface TabItem {
  id: TabId;
  labelKey: string;
  icon: React.ComponentType<{ className?: string }>;
}

const TAB_DEFS: TabItem[] = [
  { id: "content", labelKey: "nav.content", icon: ClipboardList },
  { id: "wiki", labelKey: "nav.wiki", icon: BookOpen },
  { id: "digest", labelKey: "nav.digest", icon: Target },
  { id: "datahub", labelKey: "nav.datahub", icon: HardDrive },
  { id: "settings", labelKey: "nav.settings", icon: Settings },
];

function App() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<TabId>("content");
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<CapturedContent[]>([]);
  const [wikiSearchResults, setWikiSearchResults] = useState<WikiPage[]>([]);
  const [searching, setSearching] = useState(false);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const searchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const loadFromDB = useSettingsStore((s) => s.loadFromDB);
  const isLoaded = useSettingsStore((s) => s.isLoaded);
  const setupComplete = useSettingsStore((s) => s.setupComplete);
  const setHighlightedIds = useContentStore((s) => s.setHighlightedIds);

  // Debounced search — searches both content and wiki pages
  const doSearch = useCallback((query: string) => {
    if (searchTimerRef.current) clearTimeout(searchTimerRef.current);
    if (!query.trim()) {
      setSearchResults([]);
      setWikiSearchResults([]);
      setSearching(false);
      return;
    }
    setSearching(true);
    searchTimerRef.current = setTimeout(async () => {
      try {
        const [contentResults, wikiResults] = await Promise.all([
          searchContent(query.trim()),
          searchWiki(query.trim()),
        ]);
        setSearchResults(contentResults);
        setWikiSearchResults(wikiResults);
      } catch (e) {
        console.error("Search failed:", e);
        setSearchResults([]);
        setWikiSearchResults([]);
      }
      setSearching(false);
    }, 300);
  }, []);

  // Track scroll positions per tab for restore on switch-back
  const scrollPositions = useRef<Record<TabId, number>>({
    content: 0,
    wiki: 0,
    digest: 0,
    datahub: 0,
    settings: 0,
  });

  // Load settings from database on startup
  useEffect(() => {
    loadFromDB();
  }, [loadFromDB]);

  // Save scroll position before switching away, then switch tab
  const switchTab = useCallback(
    (newTab: TabId, highlightIds?: string[]) => {
      // Save current scroll position
      scrollPositions.current[activeTab] = window.scrollY;

      // Set highlights if navigating to content with specific IDs
      if (newTab === "content" && highlightIds && highlightIds.length > 0) {
        setHighlightedIds(highlightIds);
      }

      setActiveTab(newTab);

      // Restore scroll position for the new tab
      // (skip restore if we have highlights — ContentList will handle scroll-to-item)
      if (!(newTab === "content" && highlightIds && highlightIds.length > 0)) {
        requestAnimationFrame(() => {
          window.scrollTo(0, scrollPositions.current[newTab]);
        });
      }
    },
    [activeTab, setHighlightedIds]
  );

  // Listen for tab navigation events from the tray menu
  useEffect(() => {
    const unlisten = listen<string>("navigate-tab", (event) => {
      const tab = event.payload as TabId;
      if (TAB_DEFS.some((td) => td.id === tab)) {
        switchTab(tab);
      }
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [switchTab]);

  // Listen for "navigate-to-content" events from ReportCard's "跳转原文" button
  useEffect(() => {
    const handler = (e: Event) => {
      const customEvent = e as CustomEvent<{ contentIds?: string[] }>;
      const contentIds = customEvent.detail?.contentIds ?? [];
      switchTab("content", contentIds);
    };
    window.addEventListener("navigate-to-content", handler);
    return () => window.removeEventListener("navigate-to-content", handler);
  }, [switchTab]);

  // Listen for "navigate-to-wiki-page" events from ContentCard's knowledge tags
  useEffect(() => {
    const handler = (e: Event) => {
      const customEvent = e as CustomEvent<{ pageId?: string }>;
      switchTab("wiki");
      // WikiView will pick up the page selection via store
      if (customEvent.detail?.pageId) {
        // Small delay to let WikiView mount, then select page
        setTimeout(() => {
          import("./stores/wikiStore").then(({ useWikiStore }) => {
            useWikiStore.getState().selectPage(customEvent.detail?.pageId ?? "");
          });
        }, 100);
      }
    };
    window.addEventListener("navigate-to-wiki-page", handler);
    return () => window.removeEventListener("navigate-to-wiki-page", handler);
  }, [switchTab]);

  if (!isLoaded) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-[#FAFAF8] dark:bg-[#0C0A09]">
        <div className="text-sm text-gray-500 dark:text-slate-400">{t("action.loading")}</div>
      </div>
    );
  }

  if (!setupComplete) {
    return <WorkspaceSetupView />;
  }

  return (
    <div className="min-h-screen relative overflow-hidden bg-[#FAFAF8] dark:bg-[#0C0A09] transition-colors duration-300">
      {/* Header: single-row layout, traffic lights + brand + tabs + search in one bar */}
      <header className="sticky top-0 z-10 bg-white/30 dark:bg-white/[0.03] backdrop-blur-xl border-b border-white/10 dark:border-white/[0.06]" data-tauri-drag-region>
        <div className="relative flex items-center pl-[78px] pr-4 h-[40px]" data-tauri-drag-region>
          {/* Brand — left side, after traffic lights */}
          <span className="text-base font-bold text-orange-500 flex-shrink-0" data-tauri-drag-region>
            OpenWiki
          </span>

          {/* Tab navigation — absolute center in the full header width */}
          <nav className="absolute inset-0 flex items-center justify-center pointer-events-none" data-tauri-drag-region>
            <div className="inline-flex bg-gray-100/60 dark:bg-white/[0.06] rounded-md p-0.5 pointer-events-auto">
              {TAB_DEFS.map((tab) => {
                const IconComponent = tab.icon;
                return (
                  <button
                    key={tab.id}
                    onClick={() => switchTab(tab.id)}
                    className={`
                      flex items-center gap-1 px-3 py-1 text-[13px] font-medium
                      rounded transition-all duration-200
                      ${
                        activeTab === tab.id
                          ? "bg-white dark:bg-white/[0.15] text-orange-500 dark:text-orange-400 shadow-sm"
                          : "text-gray-500 dark:text-slate-400 hover:text-gray-700 dark:hover:text-slate-300"
                      }
                    `}
                  >
                    <IconComponent className="w-3.5 h-3.5" />
                    <span>{t(tab.labelKey)}</span>
                  </button>
                );
              })}
            </div>
          </nav>

          {/* Spacer to push search to right */}
          <div className="flex-1" data-tauri-drag-region />

          {/* Search icon + expandable input */}
          <div className="flex-shrink-0 relative">
            {searchOpen ? (
              <div className="flex items-center gap-1.5">
                <div className="relative">
                  <input
                    ref={searchInputRef}
                    type="text"
                    value={searchQuery}
                    onChange={(e) => { setSearchQuery(e.target.value); doSearch(e.target.value); }}
                    onKeyDown={(e) => {
                      if (e.key === "Escape") {
                        setSearchOpen(false);
                        setSearchQuery("");
                        setSearchResults([]);
                        setWikiSearchResults([]);
                      }
                    }}
                    placeholder={t("action.search") + "..."}

                    className="w-48 px-2.5 py-1 text-xs border border-white/60 dark:border-white/[0.1] rounded-lg
                               bg-white/60 dark:bg-white/[0.06] text-gray-800 dark:text-gray-200
                               placeholder-gray-400 dark:placeholder-slate-500
                               focus:border-orange-400/60 dark:focus:border-orange-500/40
                               outline-none transition-all"
                    autoFocus
                  />
                  {/* Search results dropdown */}
                  {searchQuery.trim() && (
                    <div className="absolute right-0 top-full mt-1.5 w-80 max-h-72 overflow-y-auto
                                    bg-white/90 dark:bg-slate-800/90 backdrop-blur-xl
                                    border border-white/60 dark:border-white/[0.1]
                                    rounded-xl shadow-lg z-50">
                      {searching ? (
                        <div className="px-3 py-4 text-center text-xs text-gray-400 dark:text-slate-500">{t("action.loading")}</div>
                      ) : searchResults.length === 0 && wikiSearchResults.length === 0 ? (
                        <div className="px-3 py-4 text-center text-xs text-gray-400 dark:text-slate-500">{t("action.noData")}</div>
                      ) : (
                        <>
                          {/* Wiki results first */}
                          {wikiSearchResults.length > 0 && (
                            <>
                              <div className="px-3 py-1.5 text-[10px] font-semibold text-orange-500 bg-orange-500/5">
                                {t("wiki:title")}
                              </div>
                              {wikiSearchResults.slice(0, 3).map((wp) => (
                                <button
                                  key={`wiki-${wp.id}`}
                                  onClick={() => {
                                    switchTab("wiki");
                                    setTimeout(() => {
                                      import("./stores/wikiStore").then(({ useWikiStore }) => {
                                        useWikiStore.getState().selectPage(wp.id);
                                      });
                                    }, 100);
                                    setSearchOpen(false);
                                    setSearchQuery("");
                                    setSearchResults([]);
                                    setWikiSearchResults([]);
                                  }}
                                  className="w-full text-left px-3 py-2 hover:bg-orange-500/10 dark:hover:bg-orange-500/15
                                             border-b border-gray-100/50 dark:border-white/[0.04] transition-colors"
                                >
                                  <div className="flex items-center gap-2">
                                    <BookOpen size={12} className="flex-shrink-0 text-orange-500" />
                                    <p className="text-xs text-gray-700 dark:text-gray-200 truncate flex-1 font-medium">
                                      {wp.title}
                                    </p>
                                    <span className="text-[10px] text-orange-400 flex-shrink-0">{wp.page_type}</span>
                                  </div>
                                  {wp.summary && (
                                    <p className="text-[10px] text-gray-400 dark:text-slate-500 truncate mt-0.5 ml-5">
                                      {wp.summary}
                                    </p>
                                  )}
                                </button>
                              ))}
                            </>
                          )}
                          {/* Content results */}
                          {searchResults.length > 0 && (
                            <>
                              {wikiSearchResults.length > 0 && (
                                <div className="px-3 py-1.5 text-[10px] font-semibold text-gray-400 dark:text-slate-500 bg-gray-50/50 dark:bg-white/[0.02]">
                                  {t("content:title")}
                                </div>
                              )}
                              {searchResults.map((item) => (
                                <button
                                  key={item.id}
                                  onClick={() => {
                                    switchTab("content", [item.id]);
                                    setSearchOpen(false);
                                    setSearchQuery("");
                                    setSearchResults([]);
                                    setWikiSearchResults([]);
                                  }}
                                  className="w-full text-left px-3 py-2 hover:bg-orange-500/10 dark:hover:bg-orange-500/15
                                             border-b border-gray-100/50 dark:border-white/[0.04] last:border-0 transition-colors"
                                >
                                  <div className="flex items-center gap-2">
                                    <span className="text-xs flex-shrink-0">
                                      {item.content_type === "image" ? "📷" : item.content_type === "url" ? "🔗" : "📝"}
                                    </span>
                                    <p className="text-xs text-gray-700 dark:text-gray-200 truncate flex-1">
                                      {item.raw_text?.slice(0, 80) || item.source_url || t("content:card.noContent")}
                                    </p>
                                  </div>
                                  <div className="flex items-center gap-1.5 mt-0.5 ml-5">
                                    <span className="text-[10px] text-gray-400 dark:text-slate-500">
                                      {item.captured_at?.slice(0, 10)}
                                    </span>
                                    <span className="text-[10px] text-gray-300 dark:text-slate-600">·</span>
                                    <span className="text-[10px] text-gray-400 dark:text-slate-500">
                                      {item.source_app}
                                    </span>
                                  </div>
                                </button>
                              ))}
                            </>
                          )}
                        </>
                      )}
                    </div>
                  )}
                </div>
                <button
                  onClick={() => { setSearchOpen(false); setSearchQuery(""); setSearchResults([]); setWikiSearchResults([]); }}
                  className="p-1 text-gray-400 dark:text-slate-500 hover:text-gray-600 dark:hover:text-slate-300 transition-colors"
                >
                  <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
                  </svg>
                </button>
              </div>
            ) : (
              <button
                onClick={() => setSearchOpen(true)}
                className="p-1.5 text-gray-400 dark:text-slate-500 hover:text-orange-500 dark:hover:text-orange-400
                           hover:bg-white/50 dark:hover:bg-white/[0.08] rounded-lg transition-all"
                title={t("action.search")}
              >
                <Search className="w-4 h-4" />
              </button>
            )}
          </div>
        </div>
      </header>

      {/* Update available — shown when backend emits `update-available` on startup */}
      <UpdateBanner />

      {/* Automation permission denial banner + grant/dismiss toasts */}
      <AutomationNotices />

      {/* Tab content — relative z-index above orbs.
          All tabs stay mounted (toggled via CSS `display`) so their state
          — open chat sessions, scroll positions, form input, filter
          selections, etc. — persists across tab switches. Originally
          conditional-rendered, which wiped e.g. the WikiAskSidebar chat
          every time the user glanced at the Content tab. Scanned for
          persistent intervals / polling / global keyboard handlers
          before the switch; none exist on these tabs, so the cost of
          keeping all five mounted is just React reconciliation overhead
          (negligible at ~150 wiki pages). */}
      <main className="relative z-[1]">
        <div style={{ display: activeTab === "content" ? "block" : "none" }}>
          <ContentList />
        </div>
        <div style={{ display: activeTab === "wiki" ? "block" : "none" }}>
          <WikiView />
        </div>
        <div style={{ display: activeTab === "digest" ? "block" : "none" }}>
          <RadarView />
        </div>
        <div style={{ display: activeTab === "datahub" ? "block" : "none" }}>
          <DataHubView />
        </div>
        <div style={{ display: activeTab === "settings" ? "block" : "none" }}>
          <SettingsView />
        </div>
      </main>

      {/* First-launch Automation permission modal — fullscreen overlay */}
      <PreAuthModal />

      {/* Floating bubble is now a separate always-on-top window (BubbleView) */}
    </div>
  );
}

export default App;
