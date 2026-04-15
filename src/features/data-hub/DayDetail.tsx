import { useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { useDataHubStore } from "../../stores/dataHubStore";
import { exportDay, exportSelectedSingle, renderSelectedMarkdown } from "../../services/dataHubService";
import { compileContentsToWiki } from "../../services/wikiService";
import type { CapturedContent, ContentType } from "../../types/content";
import { ContentCard } from "../content-list/ContentCard";

const WEEKDAY_KEYS = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"] as const;

function useFormatDateHeader() {
  const { t } = useTranslation("dataHub");

  return (dateStr: string): string => {
    const d = new Date(dateStr + "T00:00:00");
    const month = d.getMonth() + 1;
    const day = d.getDate();
    const weekday = t(`weekday.${WEEKDAY_KEYS[d.getDay()]}`);
    return t("dateFormat.fullDate", { month, day, weekday });
  };
}

function useTypeConfig(): Record<ContentType, { icon: string; label: string; order: number }> {
  const { t } = useTranslation("dataHub");
  return {
    text: { icon: "📝", label: t("dayDetail.contentType.text"), order: 0 },
    url: { icon: "🔗", label: t("dayDetail.contentType.url"), order: 1 },
    image: { icon: "📷", label: t("dayDetail.contentType.image"), order: 2 },
    mixed: { icon: "📎", label: t("dayDetail.contentType.mixed"), order: 3 },
  };
}

interface ContentGroupProps {
  type: ContentType;
  items: CapturedContent[];
  selectionMode: boolean;
  selectedIds: ReadonlySet<string>;
  onToggleSelect: (id: string) => void;
}

function ContentGroup({ type, items, selectionMode, selectedIds, onToggleSelect }: ContentGroupProps) {
  const [expanded, setExpanded] = useState(true);
  const typeConfig = useTypeConfig();
  const config = typeConfig[type];

  return (
    <div className="mb-4">
      <button
        onClick={() => setExpanded(!expanded)}
        className="flex items-center gap-2 mb-2 px-1 text-sm font-medium text-gray-700 dark:text-gray-300
                   hover:text-gray-900 dark:hover:text-gray-100 transition-colors"
      >
        <span className="text-xs text-gray-400 dark:text-slate-500">
          {expanded ? "▼" : "►"}
        </span>
        <span>
          {config.icon} {config.label}
        </span>
        <span className="text-xs text-gray-400 dark:text-slate-500">
          ({items.length})
        </span>
      </button>

      {expanded && (
        <div className="space-y-2">
          {items.map((item) => (
            <ContentCard
              key={item.id}
              content={item}
              selectionMode={selectionMode}
              selected={selectedIds.has(item.id)}
              onToggleSelect={() => onToggleSelect(item.id)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

// Welcome / overview when no date is selected
function WelcomeView() {
  const { t } = useTranslation("dataHub");
  const totalDates = useDataHubStore((s) => s.totalDates);
  const totalItems = useDataHubStore((s) => s.totalItems);

  return (
    <div className="flex flex-col items-center justify-center h-full py-20">
      <div className="w-20 h-20 rounded-2xl glass flex items-center justify-center mb-6">
        <span className="text-4xl">📂</span>
      </div>
      <p className="text-lg font-medium text-gray-700 dark:text-gray-200 mb-2">
        {t("welcome.title")}
      </p>
      <p className="text-sm text-gray-400 dark:text-slate-500 mb-8">
        {t("welcome.desc")}
      </p>

      {/* Stats cards */}
      <div className="flex gap-4">
        <div className="glass rounded-2xl p-5 text-center min-w-[120px]">
          <div className="text-2xl font-bold text-orange-500">
            {totalItems}
          </div>
          <div className="text-xs text-gray-500 dark:text-slate-400 mt-1">
            {t("welcome.totalItems")}
          </div>
        </div>
        <div className="glass rounded-2xl p-5 text-center min-w-[120px]">
          <div className="text-2xl font-bold text-orange-500">
            {totalDates}
          </div>
          <div className="text-xs text-gray-500 dark:text-slate-400 mt-1">
            {t("welcome.activeDays")}
          </div>
        </div>
      </div>
    </div>
  );
}

export function DayDetail() {
  const { t } = useTranslation("dataHub");
  const formatDateHeader = useFormatDateHeader();
  const typeConfig = useTypeConfig();
  const selectedDate = useDataHubStore((s) => s.selectedDate);
  const dayContents = useDataHubStore((s) => s.dayContents);
  const isLoading = useDataHubStore((s) => s.isLoading);
  const [isExporting, setIsExporting] = useState(false);
  const [selectionMode, setSelectionMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [selectionStatus, setSelectionStatus] = useState<"idle" | "copying" | "exporting" | "compiling" | "copied" | "exported" | "compiled">("idle");
  const [compileProgress, setCompileProgress] = useState<{ current: number; total: number; compiled: number; failed: number } | null>(null);

  // Group contents by type
  const groupedContents = useMemo(() => {
    const groups = new Map<ContentType, CapturedContent[]>();
    for (const item of dayContents) {
      const t = item.content_type;
      if (!groups.has(t)) {
        groups.set(t, []);
      }
      groups.get(t)!.push(item);
    }

    // Sort groups by the defined order
    const sorted = Array.from(groups.entries()).sort(
      (a, b) =>
        (typeConfig[a[0]]?.order ?? 99) - (typeConfig[b[0]]?.order ?? 99)
    );

    return sorted;
  }, [dayContents, typeConfig]);

  useEffect(() => {
    setSelectionMode(false);
    setSelectedIds([]);
    setSelectionStatus("idle");
    setCompileProgress(null);
  }, [selectedDate]);

  useEffect(() => {
    const validIds = new Set(dayContents.map((item) => item.id));
    setSelectedIds((current) => current.filter((id) => validIds.has(id)));
  }, [dayContents]);

  useEffect(() => {
    let cancelled = false;
    const unlistenPromise = listen("wiki-compile-progress", (event) => {
      if (cancelled || selectionStatus !== "compiling") return;
      const payload = event.payload as { current?: number; total?: number; compiled?: number; failed?: number } | string;
      if (!payload || typeof payload === "string") return;
      setCompileProgress({
        current: payload.current ?? 0,
        total: payload.total ?? selectedIds.length,
        compiled: payload.compiled ?? 0,
        failed: payload.failed ?? 0,
      });
    });

    return () => {
      cancelled = true;
      unlistenPromise.then((unlisten) => unlisten()).catch(() => {});
    };
  }, [selectionStatus, selectedIds.length]);

  const selectedSet = useMemo(() => new Set(selectedIds), [selectedIds]);
  const selectedCount = selectedIds.length;
  const allSelected = dayContents.length > 0 && selectedCount === dayContents.length;

  const toggleSelected = (id: string) => {
    setSelectedIds((current) =>
      current.includes(id)
        ? current.filter((itemId) => itemId !== id)
        : [...current, id]
    );
  };

  const handleExportDay = async () => {
    if (!selectedDate) return;
    setIsExporting(true);
    try {
      await exportDay(selectedDate);
    } catch (e) {
      console.error("Failed to export:", e);
    } finally {
      setIsExporting(false);
    }
  };

  const handleToggleSelectAll = () => {
    setSelectedIds(allSelected ? [] : dayContents.map((item) => item.id));
  };

  const handleCopySelected = async () => {
    if (selectedCount === 0) return;
    setSelectionStatus("copying");
    try {
      const markdown = await renderSelectedMarkdown(selectedIds);
      await navigator.clipboard.writeText(markdown);
      setSelectionStatus("copied");
      setTimeout(() => setSelectionStatus("idle"), 2200);
    } catch (e) {
      console.error("Failed to copy selected content:", e);
      setSelectionStatus("idle");
    }
  };

  const handleExportSelected = async () => {
    if (selectedCount === 0) return;
    setSelectionStatus("exporting");
    try {
      await exportSelectedSingle(selectedIds);
      setSelectionStatus("exported");
      setTimeout(() => setSelectionStatus("idle"), 2200);
    } catch (e) {
      console.error("Failed to export selected content:", e);
      setSelectionStatus("idle");
    }
  };

  const handleCompileSelected = async () => {
    if (selectedCount === 0) return;
    setSelectionStatus("compiling");
    setCompileProgress({ current: 0, total: selectedIds.length, compiled: 0, failed: 0 });
    try {
      await compileContentsToWiki(selectedIds);
      setSelectionStatus("compiled");
      setCompileProgress((current) =>
        current
          ? { ...current, current: current.total, compiled: Math.max(current.compiled, current.total - current.failed) }
          : null
      );
      setTimeout(() => {
        setSelectionStatus("idle");
        setCompileProgress(null);
      }, 2200);
    } catch (e) {
      console.error("Failed to compile selected content to wiki:", e);
      setSelectionStatus("idle");
      setCompileProgress(null);
    }
  };

  if (!selectedDate) {
    return <WelcomeView />;
  }

  if (isLoading) {
    return (
      <div className="p-6 space-y-3">
        <div className="h-8 w-48 bg-white/50 dark:bg-white/[0.06] rounded-lg animate-pulse" />
        {[1, 2, 3].map((i) => (
          <div key={i} className="glass rounded-xl p-4">
            <div className="space-y-2">
              <div className="h-3 bg-gray-200/50 dark:bg-white/[0.06] rounded w-1/4 animate-pulse" />
              <div className="h-4 bg-gray-200/30 dark:bg-white/[0.04] rounded w-3/4 animate-pulse" />
              <div className="h-3 bg-gray-200/30 dark:bg-white/[0.04] rounded w-1/2 animate-pulse" />
            </div>
          </div>
        ))}
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      {/* Day header */}
      <div className="px-6 py-4 border-b border-white/30 dark:border-white/[0.06]">
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-lg font-semibold text-gray-800 dark:text-gray-100 flex items-center gap-2">
              <span>📅</span>
              {formatDateHeader(selectedDate)}
            </h2>
            <p className="text-xs text-gray-400 dark:text-slate-500 mt-0.5">
              {t("dayDetail.itemsCount", { count: dayContents.length })}
            </p>
          </div>
        </div>
        <div className="mt-3 flex flex-wrap items-center gap-2">
          <button
            onClick={handleExportDay}
            disabled={isExporting}
            className="flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium rounded-lg border
                       bg-white/50 dark:bg-white/[0.04] border-white/60 dark:border-white/[0.08]
                       text-gray-600 dark:text-slate-300
                       hover:bg-white/80 dark:hover:bg-white/[0.08]
                       disabled:opacity-50 disabled:cursor-not-allowed
                       transition-all duration-150"
          >
            {isExporting ? (
              <span className="animate-spin text-sm">⏳</span>
            ) : (
              <span className="text-sm">📤</span>
            )}
            <span>{t("dayDetail.exportDay")}</span>
          </button>
          <button
            onClick={() => {
              setSelectionMode((current) => !current);
              setSelectedIds([]);
              setSelectionStatus("idle");
            }}
            className={`flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium rounded-lg border transition-all duration-150
              ${selectionMode
                ? "bg-orange-500/12 dark:bg-orange-500/18 border-orange-300/50 dark:border-orange-500/30 text-orange-700 dark:text-orange-400"
                : "bg-white/50 dark:bg-white/[0.04] border-white/60 dark:border-white/[0.08] text-gray-600 dark:text-slate-300 hover:bg-white/80 dark:hover:bg-white/[0.08]"
              }`}
          >
            <span>{selectionMode ? "✓" : "☑"}</span>
            <span>{selectionMode ? t("selection.done") : t("selection.enter")}</span>
          </button>
          {selectionMode && (
            <>
              <span className="px-2.5 py-1 text-xs rounded-full bg-orange-500/10 dark:bg-orange-500/15 text-orange-700 dark:text-orange-300">
                {t("selection.count", { count: selectedCount })}
              </span>
              <button
                onClick={handleToggleSelectAll}
                className="px-3 py-1.5 text-sm font-medium rounded-lg border
                           bg-white/50 dark:bg-white/[0.04] border-white/60 dark:border-white/[0.08]
                           text-gray-600 dark:text-slate-300 hover:bg-white/80 dark:hover:bg-white/[0.08]
                           transition-all duration-150"
              >
                {allSelected ? t("selection.clearAll") : t("selection.selectAll")}
              </button>
              <button
                onClick={handleCopySelected}
                disabled={selectedCount === 0 || selectionStatus === "copying" || selectionStatus === "exporting" || selectionStatus === "compiling"}
                className="px-3 py-1.5 text-sm font-medium rounded-lg border
                           border-orange-200/50 dark:border-orange-500/20
                           bg-orange-50/50 dark:bg-orange-500/[0.06]
                           text-orange-600 dark:text-orange-400
                           hover:bg-orange-100/50 dark:hover:bg-orange-500/[0.12]
                           disabled:opacity-50 transition-all duration-150"
              >
                {selectionStatus === "copying" ? t("selection.copying") : selectionStatus === "copied" ? t("selection.copied") : t("selection.copySelected")}
              </button>
              <button
                onClick={handleExportSelected}
                disabled={selectedCount === 0 || selectionStatus === "copying" || selectionStatus === "exporting" || selectionStatus === "compiling"}
                className="px-3 py-1.5 text-sm font-medium rounded-lg border
                           border-orange-200/50 dark:border-orange-500/20
                           bg-orange-50/50 dark:bg-orange-500/[0.06]
                           text-orange-600 dark:text-orange-400
                           hover:bg-orange-100/50 dark:hover:bg-orange-500/[0.12]
                           disabled:opacity-50 transition-all duration-150"
              >
                {selectionStatus === "exporting" ? t("selection.exporting") : selectionStatus === "exported" ? t("selection.exported") : t("selection.exportSelected")}
              </button>
              <button
                onClick={handleCompileSelected}
                disabled={selectedCount === 0 || selectionStatus === "copying" || selectionStatus === "exporting" || selectionStatus === "compiling"}
                className="px-3 py-1.5 text-sm font-medium rounded-lg border
                           border-orange-200/50 dark:border-orange-500/20
                           bg-orange-50/50 dark:bg-orange-500/[0.06]
                           text-orange-600 dark:text-orange-400
                           hover:bg-orange-100/50 dark:hover:bg-orange-500/[0.12]
                           disabled:opacity-50 transition-all duration-150"
              >
                {selectionStatus === "compiling"
                  ? `${t("selection.compiling")}${compileProgress?.total ? ` ${compileProgress.current}/${compileProgress.total}` : ""}`
                  : selectionStatus === "compiled"
                    ? t("selection.compiled")
                    : t("selection.addToKnowledge")}
              </button>
            </>
          )}
        </div>
      </div>

      {/* Content area */}
      <div className="flex-1 overflow-y-auto p-6">
        {dayContents.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-16 text-center">
            <span className="text-3xl mb-3">📭</span>
            <p className="text-sm text-gray-500 dark:text-slate-400">
              {t("dayDetail.noContentForDay")}
            </p>
          </div>
        ) : (
          groupedContents.map(([type, items]) => (
            <ContentGroup
              key={type}
              type={type}
              items={items}
              selectionMode={selectionMode}
              selectedIds={selectedSet}
              onToggleSelect={toggleSelected}
            />
          ))
        )}
      </div>
    </div>
  );
}
