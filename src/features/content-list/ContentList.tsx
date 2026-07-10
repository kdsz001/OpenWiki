import {
  useEffect,
  useCallback,
  useState,
  useMemo,
  useRef,
  type ChangeEvent,
} from "react";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { CheckCircle2, FileText, FolderOpen, Image as ImageIcon, Import, Link2, LoaderCircle, Upload, XCircle } from "lucide-react";
import { useContentStore } from "../../stores/contentStore";
import {
  getAllContent,
  getStorageInfo,
  getContentsByIds,
  importContentFiles,
  importUrls,
  type ContentImportEntry,
  type ContentImportKind,
  type UrlImportEntry,
  type UrlImportProgressEvent,
} from "../../services/storageService";
import { exportAllSingle, exportRangeSingle } from "../../services/dataHubService";
import { useSettingsStore, containsSensitiveData } from "../../stores/settingsStore";
import { ContentCard } from "./ContentCard";
import type { ContentType } from "../../types/content";

type FilterType = "all" | ContentType;
type DateRange = "all" | "today" | "week" | "half-month";
type ContentFilter = FilterType | "document";
type ImportStatus = "idle" | "picking" | "reading" | "converting" | "saving" | "done" | "error";

const FILTER_TABS: { value: ContentFilter; labelKey: string; icon: string }[] = [
  { value: "all", labelKey: "filter.all", icon: "📋" },
  { value: "text", labelKey: "filter.text", icon: "📝" },
  { value: "image", labelKey: "filter.image", icon: "🖼️" },
  { value: "url", labelKey: "filter.url", icon: "🔗" },
  { value: "document", labelKey: "filter.document", icon: "📥" },
];

const IMPORT_ACCEPT = [
  ".md",
  ".markdown",
  ".txt",
  ".png",
  ".jpg",
  ".jpeg",
  ".webp",
  ".gif",
  ".pdf",
  ".docx",
  ".pptx",
  "text/markdown",
  "text/x-markdown",
  "text/plain",
  "image/png",
  "image/jpeg",
  "image/webp",
  "image/gif",
  "application/pdf",
  "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  "application/vnd.openxmlformats-officedocument.presentationml.presentation",
].join(",");

const IMPORT_SOURCE_APPS = new Set(["Markdown 导入", "导入内容"]);
const SUPPORTED_IMPORT_FORMATS = ["Markdown", "TXT", "PNG", "JPG", "WebP", "GIF", "PDF", "DOCX", "PPTX"];
const FUTURE_IMPORT_FORMATS = ["DOC", "PPT"];
const LONG_IMPORT_NOTICE_MS = 8000;
// Import folders in chunks so a large library doesn't read every file into
// memory at once (or ship one huge IPC payload), and so we can show progress.
const IMPORT_BATCH_SIZE = 20;
const URL_IMPORT_SAMPLE_LIMIT = 4;
const BOOKMARK_IMPORT_ACCEPT = ".html,.htm,text/html";

const isImportedDocument = (content: { source_app: string }) =>
  IMPORT_SOURCE_APPS.has(content.source_app);

const getImportKind = (file: File): ContentImportKind | null => {
  const name = file.name.toLowerCase();
  if (name.endsWith(".md") || name.endsWith(".markdown")) return "markdown";
  if (name.endsWith(".txt")) return "text";
  if (name.endsWith(".pdf") || name.endsWith(".docx") || name.endsWith(".pptx")) return "document";
  if (
    file.type.startsWith("image/") ||
    name.endsWith(".png") ||
    name.endsWith(".jpg") ||
    name.endsWith(".jpeg") ||
    name.endsWith(".webp") ||
    name.endsWith(".gif")
  ) {
    return "image";
  }
  return null;
};

const readFileAsBase64 = (file: File) => new Promise<string>((resolve, reject) => {
  const reader = new FileReader();
  reader.onload = () => {
    const result = typeof reader.result === "string" ? reader.result : "";
    const data = result.includes(",") ? result.split(",")[1] : result;
    if (data) {
      resolve(data);
    } else {
      reject(new Error("Empty file data"));
    }
  };
  reader.onerror = () => reject(reader.error ?? new Error("Failed to read file"));
  reader.readAsDataURL(file);
});

const normalizeUrl = (value: string): string | null => {
  const trimmed = value
    .trim()
    .replace(/^[<('"[]+/, "")
    .replace(/[>)'".,\]]+$/, "");

  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return null;
    parsed.hash = "";
    return parsed.href;
  } catch {
    return null;
  }
};

const extractUrlsFromText = (text: string): string[] => {
  const urls = new Set<string>();
  const maybeHtml = /<a\s/i.test(text) || /<!doctype\s+net/i.test(text);
  let searchableText = text;

  if (maybeHtml) {
    const doc = new DOMParser().parseFromString(text, "text/html");
    doc.querySelectorAll("a[href]").forEach((anchor) => {
      const url = normalizeUrl(anchor.getAttribute("href") ?? "");
      if (url) urls.add(url);
    });
    searchableText = doc.body.textContent ?? "";
  }

  const matches = searchableText.match(/https?:\/\/[^\s<>"']+/gi) ?? [];
  matches.forEach((match) => {
    const url = normalizeUrl(match);
    if (url) urls.add(url);
  });

  return Array.from(urls);
};

const PAGE_SIZE = 50;

export function ContentList() {
  const { t } = useTranslation("content");
  const { contents, isLoading, setContents, setIsLoading } = useContentStore();
  const hasMore = useContentStore((s) => s.hasMore);
  const totalCount = useContentStore((s) => s.totalCount);
  const isLoadingMore = useContentStore((s) => s.isLoadingMore);
  const setHasMore = useContentStore((s) => s.setHasMore);
  const setTotalCount = useContentStore((s) => s.setTotalCount);
  const setIsLoadingMore = useContentStore((s) => s.setIsLoadingMore);
  const appendContents = useContentStore((s) => s.appendContents);
  const highlightedIds = useContentStore((s) => s.highlightedIds);
  const scrollToId = useContentStore((s) => s.scrollToId);
  const setScrollToId = useContentStore((s) => s.setScrollToId);
  const clearHighlights = useContentStore((s) => s.clearHighlights);
  const setHighlightedIds = useContentStore((s) => s.setHighlightedIds);
  const captureEnabled = useSettingsStore((s) => s.captureEnabled);
  const sensitiveFilterEnabled = useSettingsStore((s) => s.sensitiveFilterEnabled);
  const setStorageInfo = useSettingsStore((s) => s.setStorageInfo);
  const [filter, setFilter] = useState<ContentFilter>("all");
  const [dateRange, setDateRange] = useState<DateRange>("all");
  const [exportStatus, setExportStatus] = useState<"idle" | "confirm" | "exporting" | "done">("idle");
  const [importStatus, setImportStatus] = useState<ImportStatus>("idle");
  const [importMessage, setImportMessage] = useState("");
  const [isImportTakingLong, setIsImportTakingLong] = useState(false);
  const [isImportPanelOpen, setIsImportPanelOpen] = useState(false);
  const [isUrlImportOpen, setIsUrlImportOpen] = useState(false);
  const [urlImportText, setUrlImportText] = useState("");
  const confirmTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const importInputRef = useRef<HTMLInputElement>(null);
  const folderInputRef = useRef<HTMLInputElement>(null);
  const bookmarkInputRef = useRef<HTMLInputElement>(null);
  const importPickerOpenRef = useRef(false);
  const importPanelRef = useRef<HTMLDivElement>(null);
  const activeUrlImportJobRef = useRef<string | null>(null);
  const urlImportStatusTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Refs for scroll-to-item and infinite scroll sentinel
  const cardRefs = useRef<Record<string, HTMLDivElement | null>>({});
  const scrollContainerRef = useRef<HTMLDivElement>(null);

  // Load initial page (first 50 items)
  const loadInitial = useCallback(async () => {
    setIsLoading(true);
    try {
      const info = await getStorageInfo();
      setStorageInfo(info.total_items, info.disk_usage_mb);
      setTotalCount(info.total_items);
      const data = await getAllContent(PAGE_SIZE, 0);
      setContents(data);
      setHasMore(data.length < info.total_items);
    } catch (e) {
      console.error("Failed to load content:", e);
    } finally {
      setIsLoading(false);
    }
  }, [setContents, setIsLoading, setStorageInfo, setTotalCount, setHasMore]);

  // Load more items (append next batch)
  const loadMore = useCallback(async () => {
    if (isLoadingMore || !hasMore) return;
    setIsLoadingMore(true);
    try {
      const offset = contents.length;
      const data = await getAllContent(PAGE_SIZE, offset);
      appendContents(data);
      if (data.length < PAGE_SIZE) setHasMore(false);
    } catch (e) {
      console.error("Failed to load more:", e);
    } finally {
      setIsLoadingMore(false);
    }
  }, [contents.length, isLoadingMore, hasMore, appendContents, setIsLoadingMore, setHasMore]);

  useEffect(() => {
    loadInitial();
  }, [loadInitial]);

  const openImportPanel = useCallback(() => {
    setIsImportPanelOpen((open) => !open);
  }, []);

  const handleChooseFiles = useCallback(() => {
    if (!importInputRef.current) {
      setImportStatus("error");
      setImportMessage(t("import.failed"));
      setTimeout(() => setImportStatus("idle"), 3000);
      return;
    }
    importPickerOpenRef.current = true;
    setImportStatus("picking");
    setImportMessage(t("import.choosing"));
    setIsImportPanelOpen(false);
    importInputRef.current.click();
  }, [t]);

  // webkitdirectory isn't in React's input typings; set it on the node so the
  // native picker selects a whole folder (recursively) instead of a single file.
  const setFolderInputEl = useCallback((el: HTMLInputElement | null) => {
    folderInputRef.current = el;
    if (el) {
      el.setAttribute("webkitdirectory", "");
      el.setAttribute("directory", "");
    }
  }, []);

  const handleChooseFolder = useCallback(() => {
    if (!folderInputRef.current) {
      setImportStatus("error");
      setImportMessage(t("import.failed"));
      setTimeout(() => setImportStatus("idle"), 3000);
      return;
    }
    importPickerOpenRef.current = true;
    setImportStatus("picking");
    setImportMessage(t("import.choosing"));
    setIsImportPanelOpen(false);
    folderInputRef.current.click();
  }, [t]);

  const parsedImportUrls = useMemo(() => extractUrlsFromText(urlImportText), [urlImportText]);

  const importUrlEntries = useCallback(async (urls: string[]) => {
    if (urls.length === 0) {
      setImportStatus("error");
      setImportMessage(t("import.urlEmpty"));
      setTimeout(() => setImportStatus("idle"), 3000);
      return;
    }

    if (urlImportStatusTimerRef.current) {
      clearTimeout(urlImportStatusTimerRef.current);
      urlImportStatusTimerRef.current = null;
    }

    const entries: UrlImportEntry[] = urls.map((url) => ({ url }));
    const jobId = crypto.randomUUID();
    activeUrlImportJobRef.current = jobId;
    setImportStatus("saving");
    setImportMessage(t("import.urlQueued", { count: entries.length }));
    setIsImportPanelOpen(false);
    setIsImportTakingLong(false);

    try {
      const queued = await importUrls(entries, jobId);
      if (activeUrlImportJobRef.current === jobId) {
        setImportMessage(t("import.urlQueuedBatch", {
          count: queued.total,
          batchSize: queued.batch_size,
        }));
      }
      setUrlImportText("");
      setIsUrlImportOpen(false);
    } catch (error) {
      console.error("Failed to queue URL import:", error);
      activeUrlImportJobRef.current = null;
      setImportStatus("error");
      setImportMessage(t("import.failedWithReason", { reason: String(error) }));
      setTimeout(() => setImportStatus("idle"), 4000);
    }
  }, [t]);

  const handleImportUrls = useCallback(async () => {
    await importUrlEntries(parsedImportUrls);
  }, [importUrlEntries, parsedImportUrls]);

  const handleChooseBookmarkFile = useCallback(() => {
    if (!bookmarkInputRef.current) {
      setImportStatus("error");
      setImportMessage(t("import.failed"));
      setTimeout(() => setImportStatus("idle"), 3000);
      return;
    }

    importPickerOpenRef.current = true;
    setImportStatus("picking");
    setImportMessage(t("import.choosing"));
    bookmarkInputRef.current.click();
  }, [t]);

  const handleBookmarkImport = useCallback(async (event: ChangeEvent<HTMLInputElement>) => {
    importPickerOpenRef.current = false;
    const file = event.target.files?.[0];
    event.target.value = "";

    if (!file) {
      setImportStatus("idle");
      setImportMessage("");
      return;
    }

    try {
      setImportStatus("reading");
      setImportMessage(t("import.reading", { count: 1 }));
      await importUrlEntries(extractUrlsFromText(await file.text()));
    } catch (error) {
      console.error("Failed to read bookmark file:", error);
      setImportStatus("error");
      setImportMessage(t("import.failed"));
      setTimeout(() => setImportStatus("idle"), 4000);
    }
  }, [importUrlEntries, t]);

  useEffect(() => {
    const unlisten = listen<UrlImportProgressEvent>(
      "content:url-import-progress",
      async ({ payload }) => {
        if (activeUrlImportJobRef.current !== payload.job_id) return;

        const skipped = payload.skipped_duplicates + payload.skipped_invalid;
        if (payload.done) {
          if (payload.imported === 0 && payload.failed > 0) {
            setImportStatus("error");
            setImportMessage(t("import.failedWithReason", {
              reason: payload.first_failure ?? "",
            }));
          } else {
            setImportStatus("done");
            setImportMessage(t("import.urlDone", {
              imported: payload.imported,
              skipped,
              failed: payload.failed,
            }));
          }

          await loadInitial();
          setFilter("url");
          if (payload.imported_ids.length > 0) {
            setHighlightedIds(payload.imported_ids);
          }
          activeUrlImportJobRef.current = null;
          urlImportStatusTimerRef.current = setTimeout(() => {
            setImportStatus("idle");
            setImportMessage("");
            urlImportStatusTimerRef.current = null;
          }, 6000);
          return;
        }

        setImportStatus("saving");
        setImportMessage(t("import.urlProgress", {
          done: payload.processed,
          total: payload.total,
          imported: payload.imported,
          skipped,
          failed: payload.failed,
        }));
      }
    );

    return () => {
      unlisten.then((fn) => fn());
      if (urlImportStatusTimerRef.current) {
        clearTimeout(urlImportStatusTimerRef.current);
        urlImportStatusTimerRef.current = null;
      }
    };
  }, [loadInitial, setHighlightedIds, t]);

  const importFiles = useCallback(async (files: File[]) => {
    if (files.length === 0) {
      setImportStatus("idle");
      setImportMessage("");
      return;
    }

    const supportedFiles = files
      .map((file) => ({ file, kind: getImportKind(file) }))
      .filter((item): item is { file: File; kind: ContentImportKind } => item.kind !== null);

    if (supportedFiles.length === 0) {
      setImportStatus("error");
      setImportMessage(t("import.unsupported"));
      setTimeout(() => setImportStatus("idle"), 3000);
      return;
    }

    const total = supportedFiles.length;
    const isBatched = total > IMPORT_BATCH_SIZE;
    setIsImportTakingLong(false);
    setIsImportPanelOpen(false);

    let importedCount = 0;
    let skippedCount = 0;
    let failedCount = 0;
    let firstFailure: string | null = null;
    const allImportedIds: string[] = [];

    try {
      for (let start = 0; start < supportedFiles.length; start += IMPORT_BATCH_SIZE) {
        const batch = supportedFiles.slice(start, start + IMPORT_BATCH_SIZE);
        const hasDocument = batch.some(({ kind }) => kind === "document");

        if (isBatched) {
          // Folder / large import: show running progress (done / total).
          setImportStatus("saving");
          setImportMessage(t("import.importingProgress", { done: start, total }));
        } else {
          // Small selection: keep the original single-shot messages.
          setImportStatus(hasDocument ? "converting" : "reading");
          setImportMessage(hasDocument ? t("import.converting") : t("import.reading", { count: total }));
        }

        const entries = await Promise.all(
          batch.map(async ({ file, kind }): Promise<ContentImportEntry> => {
            if (kind === "image" || kind === "document") {
              return {
                file_name: file.name,
                kind,
                data_base64: await readFileAsBase64(file),
              };
            }
            return {
              file_name: file.name,
              kind,
              text: await file.text(),
            };
          })
        );

        if (!isBatched) {
          setImportStatus("saving");
          setImportMessage(t("import.saving"));
        }

        const result = await importContentFiles(entries);
        importedCount += result.imported.length;
        skippedCount += result.skipped_duplicates + result.skipped_invalid;
        failedCount += result.failed.length;
        if (!firstFailure && result.failed.length > 0) {
          firstFailure = result.failed[0];
        }
        allImportedIds.push(...result.imported.map((item) => item.id));
      }

      await loadInitial();
      if (allImportedIds.length > 0) {
        setHighlightedIds(allImportedIds);
      }

      if (importedCount === 0 && failedCount > 0) {
        setImportStatus("error");
        setImportMessage(t("import.failedWithReason", { reason: firstFailure ?? "" }));
      } else {
        setImportStatus("done");
        setImportMessage(t("import.done", {
          imported: importedCount,
          skipped: skippedCount,
          failed: failedCount,
        }));
      }
      setTimeout(() => setImportStatus("idle"), 4000);
    } catch (e) {
      console.error("Failed to import content:", e);
      setImportStatus("error");
      setImportMessage(t("import.failed"));
      setTimeout(() => setImportStatus("idle"), 4000);
    }
  }, [loadInitial, setHighlightedIds, t]);

  useEffect(() => {
    const isProcessing = importStatus === "reading" || importStatus === "converting" || importStatus === "saving";
    if (!isProcessing) {
      setIsImportTakingLong(false);
      return;
    }

    const timer = setTimeout(() => setIsImportTakingLong(true), LONG_IMPORT_NOTICE_MS);
    return () => clearTimeout(timer);
  }, [importStatus]);

  const handleContentImport = useCallback(async (event: ChangeEvent<HTMLInputElement>) => {
    importPickerOpenRef.current = false;
    const files = Array.from(event.target.files ?? []);
    event.target.value = "";
    await importFiles(files);
  }, [importFiles]);

  useEffect(() => {
    if (!isImportPanelOpen) return;
    const handlePointerDown = (event: MouseEvent) => {
      if (!importPanelRef.current?.contains(event.target as Node)) {
        setIsImportPanelOpen(false);
      }
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setIsImportPanelOpen(false);
      }
    };
    document.addEventListener("mousedown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [isImportPanelOpen]);

  useEffect(() => {
    const handleFocus = () => {
      if (importPickerOpenRef.current) {
        setTimeout(() => {
          if (importPickerOpenRef.current) {
            importPickerOpenRef.current = false;
            setImportStatus("idle");
            setImportMessage("");
          }
        }, 1200);
        return;
      }
      loadInitial();
    };
    window.addEventListener("focus", handleFocus);
    return () => { window.removeEventListener("focus", handleFocus); };
  }, [loadInitial]);

  // Scroll listener: trigger loadMore when near bottom of scroll container
  useEffect(() => {
    const el = scrollContainerRef.current;
    if (!el || !hasMore) return;
    const handleScroll = () => {
      if (el.scrollTop + el.clientHeight >= el.scrollHeight - 300) {
        loadMore();
      }
    };
    el.addEventListener("scroll", handleScroll, { passive: true });
    return () => el.removeEventListener("scroll", handleScroll);
  }, [hasMore, loadMore]);

  // Listen for content updates — reload single item instead of full list
  const reloadSingleItem = useCallback(async (id: string) => {
    if (!id) return;
    try {
      const items = await getContentsByIds([id]);
      if (items.length > 0) {
        useContentStore.getState().updateContent(items[0]);
      }
    } catch (e) { console.error("Failed to reload item:", e); }
  }, []);

  useEffect(() => {
    const unlisten = listen<{ id: string; reorder?: boolean }>(
      "content:url-fetched",
      (event) => { reloadSingleItem(event.payload.id); }
    );
    return () => { unlisten.then((fn) => fn()); };
  }, [reloadSingleItem]);

  useEffect(() => {
    const unlisten = listen<string>(
      "content:clean-ready",
      (event) => { reloadSingleItem(event.payload); }
    );
    return () => { unlisten.then((fn) => fn()); };
  }, [reloadSingleItem]);

  useEffect(() => {
    const unlisten = listen<string>(
      "content-summary-ready",
      (event) => { reloadSingleItem(event.payload); }
    );
    return () => { unlisten.then((fn) => fn()); };
  }, [reloadSingleItem]);

  useEffect(() => {
    const unlisten = listen<{ id: string }>(
      "content:ocr-done",
      (event) => { reloadSingleItem(event.payload.id); }
    );
    return () => { unlisten.then((fn) => fn()); };
  }, [reloadSingleItem]);

  // Handle scroll-to-item when scrollToId changes
  useEffect(() => {
    if (!scrollToId) return;

    // Reset filters to "all" so the target item is visible
    setFilter("all");

    // Wait for render, then scroll to the item
    const timer = setTimeout(() => {
      const el = cardRefs.current[scrollToId];
      if (el) {
        el.scrollIntoView({ behavior: "smooth", block: "center" });
        setScrollToId(null);
      }
    }, 150);

    return () => clearTimeout(timer);
  }, [scrollToId, setScrollToId, contents]);

  // Auto-clear highlights after 4 seconds
  useEffect(() => {
    if (highlightedIds.length === 0) return;
    const timer = setTimeout(() => {
      clearHighlights();
    }, 4000);
    return () => clearTimeout(timer);
  }, [highlightedIds, clearHighlights]);

  const filteredContents = useMemo(() => {
    let result = contents;
    if (sensitiveFilterEnabled) {
      result = result.filter((c) => !c.raw_text || !containsSensitiveData(c.raw_text));
    }
    if (filter === "document") {
      result = result.filter(isImportedDocument);
    } else if (filter !== "all") {
      result = result.filter((c) => c.content_type === filter && !isImportedDocument(c));
    }
    if (dateRange !== "all") {
      const now = new Date();
      const cutoff = new Date();
      if (dateRange === "today") {
        cutoff.setHours(0, 0, 0, 0);
      } else if (dateRange === "week") {
        cutoff.setDate(now.getDate() - 7);
      } else if (dateRange === "half-month") {
        cutoff.setDate(now.getDate() - 15);
      }
      result = result.filter((c) => new Date(c.captured_at) >= cutoff);
    }
    return result;
  }, [contents, filter, sensitiveFilterEnabled, dateRange]);

  const typeCounts = useMemo(() => {
    const counts: Record<string, number> = { all: totalCount };
    for (const c of contents) {
      if (isImportedDocument(c)) {
        counts.document = (counts.document || 0) + 1;
      } else {
        counts[c.content_type] = (counts[c.content_type] || 0) + 1;
      }
    }
    return counts;
  }, [contents, totalCount]);

  const isImportBusy = importStatus === "picking" || importStatus === "reading" || importStatus === "converting" || importStatus === "saving";

  const renderImportPanel = (align: "center" | "right") => {
    if (!isImportPanelOpen) return null;
    return (
      <div
        className={`absolute top-full mt-2 z-50 max-h-[min(520px,calc(100vh-260px))] w-80 overflow-y-auto rounded-xl border border-stone-200/80 bg-white p-3 shadow-xl shadow-stone-950/10 dark:border-white/[0.08] dark:bg-stone-950 dark:shadow-black/30 ${
          align === "center" ? "left-1/2 -translate-x-1/2" : "right-0"
        }`}
      >
        <div className="flex items-center justify-between mb-3">
          <div className="flex items-center gap-2 text-sm font-semibold text-stone-800 dark:text-stone-100">
            <Import size={16} className="text-orange-500" />
            {t("import.panelTitle")}
          </div>
        </div>

        <div className="space-y-2">
          <div>
            <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-medium text-stone-500 dark:text-stone-400">
              <FileText size={13} />
              {t("import.supportedLabel")}
            </div>
            <div className="flex flex-wrap gap-1.5">
              {SUPPORTED_IMPORT_FORMATS.map((format) => (
                <span
                  key={format}
                  className="rounded-md border border-orange-500/20 bg-orange-50 px-2 py-0.5 text-[11px] font-medium text-orange-600 dark:border-orange-400/20 dark:bg-orange-500/10 dark:text-orange-300"
                >
                  {format}
                </span>
              ))}
            </div>
          </div>

          <div>
            <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-medium text-stone-400 dark:text-stone-500">
              <ImageIcon size={13} />
              {t("import.futureLabel")}
            </div>
            <div className="flex flex-wrap gap-1.5">
              {FUTURE_IMPORT_FORMATS.map((format) => (
                <span
                  key={format}
                  className="rounded-md border border-stone-200 bg-stone-50 px-2 py-0.5 text-[11px] font-medium text-stone-400 dark:border-white/[0.08] dark:bg-white/[0.03] dark:text-stone-500"
                >
                  {format}
                </span>
              ))}
            </div>
          </div>
        </div>

        <input
          ref={bookmarkInputRef}
          type="file"
          accept={BOOKMARK_IMPORT_ACCEPT}
          className="hidden"
          onChange={handleBookmarkImport}
        />
        <div className="mt-3 rounded-lg border border-stone-200 bg-stone-50/80 p-2.5 dark:border-white/[0.08] dark:bg-white/[0.03]">
          <button
            type="button"
            onClick={() => setIsUrlImportOpen((open) => !open)}
            aria-expanded={isUrlImportOpen}
            className="flex w-full items-center justify-between gap-3 text-left text-sm font-semibold text-stone-800 dark:text-stone-100"
          >
            <span className="flex items-center gap-2">
              <Link2 size={15} className="text-orange-500" />
              {t("import.urlTitle")}
            </span>
            <span className="shrink-0 text-[11px] font-medium text-stone-400 dark:text-stone-500">
              {parsedImportUrls.length > 0
                ? t("import.urlCount", { count: parsedImportUrls.length })
                : t(isUrlImportOpen ? "import.urlCollapse" : "import.urlToggle")}
            </span>
          </button>

          {isUrlImportOpen && (
            <div className="mt-2 space-y-2">
              <textarea
                value={urlImportText}
                onChange={(event) => setUrlImportText(event.target.value)}
                aria-label={t("import.urlTitle")}
                placeholder={t("import.urlPlaceholder")}
                className="h-24 w-full resize-none rounded-md border border-stone-200 bg-white px-2.5 py-2 text-xs leading-5 text-stone-700 outline-none transition focus:border-orange-400 dark:border-white/[0.08] dark:bg-stone-900 dark:text-stone-200"
              />
              {parsedImportUrls.length > 0 && (
                <div className="space-y-1 rounded-md bg-white px-2 py-1.5 dark:bg-stone-900">
                  {parsedImportUrls.slice(0, URL_IMPORT_SAMPLE_LIMIT).map((url) => (
                    <div key={url} className="truncate text-[11px] leading-5 text-stone-500 dark:text-stone-400">
                      {url}
                    </div>
                  ))}
                  {parsedImportUrls.length > URL_IMPORT_SAMPLE_LIMIT && (
                    <div className="text-[11px] leading-5 text-stone-400 dark:text-stone-500">
                      {t("import.urlMore", { count: parsedImportUrls.length - URL_IMPORT_SAMPLE_LIMIT })}
                    </div>
                  )}
                </div>
              )}
              <div className="grid grid-cols-2 gap-2">
                <button
                  type="button"
                  onClick={handleChooseBookmarkFile}
                  disabled={isImportBusy}
                  className="flex items-center justify-center gap-1.5 rounded-md border border-stone-200 bg-white px-2 py-1.5 text-xs font-medium text-stone-600 transition hover:border-orange-300 hover:text-orange-500 disabled:opacity-60 dark:border-white/[0.08] dark:bg-stone-900 dark:text-stone-300"
                >
                  <Upload size={13} />
                  {t("import.urlFileButton")}
                </button>
                <button
                  type="button"
                  onClick={handleImportUrls}
                  disabled={isImportBusy || parsedImportUrls.length === 0}
                  className="flex items-center justify-center gap-1.5 rounded-md border border-orange-500 bg-orange-500 px-2 py-1.5 text-xs font-medium text-white transition hover:bg-orange-600 disabled:opacity-60"
                >
                  <Link2 size={13} />
                  {t("import.urlImportButton")}
                </button>
              </div>
            </div>
          )}
        </div>

        <button
          type="button"
          onClick={handleChooseFiles}
          disabled={isImportBusy}
          className="mt-3 flex w-full items-center justify-center gap-2 rounded-lg border border-orange-500 bg-orange-500 px-3 py-2 text-sm font-medium text-white transition-all hover:bg-orange-600 disabled:opacity-60"
        >
          <Import size={16} />
          {importStatus === "picking" ? t("import.choosing") : isImportBusy ? t("import.importing") : t("import.chooseButton")}
        </button>
        <button
          type="button"
          onClick={handleChooseFolder}
          disabled={isImportBusy}
          className="mt-2 flex w-full items-center justify-center gap-2 rounded-lg border border-orange-500/60 bg-transparent px-3 py-2 text-sm font-medium text-orange-600 transition-all hover:bg-orange-50 disabled:opacity-60 dark:text-orange-300 dark:hover:bg-orange-500/10"
        >
          <FolderOpen size={16} />
          {t("import.chooseFolderButton")}
        </button>
      </div>
    );
  };

  const renderImportNotice = () => {
    if (importStatus === "idle" || importStatus === "picking" || !importMessage) return null;

    const isError = importStatus === "error";
    const isDone = importStatus === "done";
    const statusColor = isError
      ? "text-red-500"
      : isDone
      ? "text-green-600 dark:text-green-400"
      : "text-orange-500";
    const borderColor = isError
      ? "border-red-200 bg-red-50 dark:border-red-500/20 dark:bg-red-500/10"
      : isDone
      ? "border-green-200 bg-green-50 dark:border-green-500/20 dark:bg-green-500/10"
      : "border-orange-200 bg-orange-50 dark:border-orange-500/20 dark:bg-orange-500/10";

    return (
      <div className="fixed bottom-6 right-6 z-[70] w-[min(360px,calc(100vw-48px))]">
        <div className={`rounded-xl border px-4 py-3 shadow-xl shadow-stone-950/10 dark:shadow-black/30 ${borderColor}`}>
          <div className="flex items-start gap-3">
            <div className={`mt-0.5 ${statusColor}`}>
              {isError ? (
                <XCircle size={18} />
              ) : isDone ? (
                <CheckCircle2 size={18} />
              ) : (
                <LoaderCircle size={18} className="animate-spin" />
              )}
            </div>
            <div className="min-w-0">
              <div className={`text-sm font-semibold ${statusColor}`}>
                {isError ? t("import.noticeError") : isDone ? t("import.noticeDone") : t("import.noticeWorking")}
              </div>
              <div className="mt-0.5 break-words text-xs leading-5 text-stone-600 dark:text-stone-300">
                {importMessage}
              </div>
              {isImportTakingLong && isImportBusy && (
                <div className="mt-1 text-[11px] leading-5 text-stone-500 dark:text-stone-400">
                  {t("import.takingLong")}
                </div>
              )}
              {isImportBusy && (
                <div className="mt-2 h-1 overflow-hidden rounded-full bg-white/80 dark:bg-white/[0.08]">
                  <div className="h-full w-1/2 rounded-full bg-orange-500/80 animate-pulse" />
                </div>
              )}
            </div>
          </div>
        </div>
      </div>
    );
  };

  if (isLoading) {
    return (
      <div className="p-4 space-y-3">
        {renderImportNotice()}
        <div className="flex items-center justify-between px-1">
          <div className="h-6 w-32 bg-white/50 dark:bg-white/[0.06] rounded-lg animate-pulse" />
          <div className="h-5 w-16 bg-white/50 dark:bg-white/[0.06] rounded-full animate-pulse" />
        </div>
        {[1, 2, 3].map((i) => (
          <div key={i} className="glass rounded-2xl p-4">
            <div className="flex items-start gap-3">
              <div className="w-8 h-8 bg-orange-500/10 dark:bg-orange-500/10 rounded-xl animate-pulse" />
              <div className="flex-1 space-y-2">
                <div className="h-4 bg-gray-200/50 dark:bg-white/[0.06] rounded w-3/4 animate-pulse" />
                <div className="h-3 bg-gray-200/30 dark:bg-white/[0.04] rounded w-1/2 animate-pulse" />
                <div className="h-3 bg-gray-200/30 dark:bg-white/[0.04] rounded w-1/3 animate-pulse" />
              </div>
            </div>
          </div>
        ))}
      </div>
    );
  }

  if (contents.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-80">
        {renderImportNotice()}
        <input
          ref={importInputRef}
          type="file"
          accept={IMPORT_ACCEPT}
          multiple
          className="hidden"
          onChange={handleContentImport}
        />
        <input
          ref={setFolderInputEl}
          type="file"
          multiple
          className="hidden"
          onChange={handleContentImport}
        />
        <div className="w-20 h-20 rounded-2xl glass flex items-center justify-center mb-5">
          <span className="text-4xl">📭</span>
        </div>
        <div className="font-medium text-gray-600 dark:text-slate-300 mb-2">
          {t("emptyTitle")}
        </div>
        <div className="text-sm text-gray-400 dark:text-slate-500 text-center max-w-xs">
          {t("emptyHint")}
        </div>
        <div ref={importPanelRef} className="relative mt-5">
          <button
            onClick={openImportPanel}
            disabled={isImportBusy}
            className="inline-flex items-center gap-2 px-3.5 py-2 rounded-lg text-sm font-medium transition-all border disabled:opacity-60"
            style={{
              color: "#F97316",
              backgroundColor: "#FFF7ED",
              borderColor: "#F9731630",
            }}
          >
            <Import size={16} />
            {isImportBusy ? t("import.importing") : t("import.button")}
          </button>
          {renderImportPanel("center")}
        </div>
        {importStatus !== "idle" && importMessage && (
          <div className={`mt-2 text-xs ${importStatus === "error" ? "text-red-500" : "text-stone-400 dark:text-stone-500"}`}>
            {importMessage}
          </div>
        )}
        <div className="mt-4 flex items-center gap-1.5 text-xs">
          <span className={`w-2 h-2 rounded-full ${captureEnabled ? "bg-green-400 animate-pulse" : "bg-gray-300 dark:bg-slate-600"}`} />
          <span className="text-gray-400 dark:text-slate-500">
            {captureEnabled ? t("captureOn") : t("captureOff")}
          </span>
        </div>
      </div>
    );
  }

  return (
    <div ref={scrollContainerRef} className="overflow-y-auto p-4 space-y-3" style={{ height: "calc(100vh - 44px)" }}>
      {renderImportNotice()}
      <input
        ref={importInputRef}
        type="file"
        accept={IMPORT_ACCEPT}
        multiple
        className="hidden"
        onChange={handleContentImport}
      />
      <input
        ref={setFolderInputEl}
        type="file"
        multiple
        className="hidden"
        onChange={handleContentImport}
      />
      {/* Header with filter tabs */}
      <div className="flex items-center justify-between px-1">
        <div className="flex items-center gap-1 p-0.5 rounded-xl glass">
          {FILTER_TABS.map((tab) => {
            const count = typeCounts[tab.value] || 0;
            if (tab.value !== "all" && count === 0) return null;
            const isActive = filter === tab.value;
            return (
              <button
                key={tab.value}
                onClick={() => setFilter(tab.value)}
                className={`
                  flex items-center gap-1 px-2.5 py-1.5 text-xs font-medium rounded-lg transition-all
                  ${isActive
                    ? "bg-white/80 dark:bg-white/[0.1] text-orange-600 dark:text-orange-400 shadow-sm"
                    : "text-gray-500 dark:text-slate-400 hover:text-gray-700 dark:hover:text-slate-300"
                  }
                `}
              >
                <span className="text-sm">{tab.icon}</span>
                <span>{t(tab.labelKey)}</span>
                <span className={`
                  ml-0.5 px-1.5 py-0.5 rounded-full text-[10px]
                  ${isActive
                    ? "bg-orange-500/10 dark:bg-orange-500/20 text-orange-600 dark:text-orange-400"
                    : "bg-gray-200/50 dark:bg-white/[0.06] text-gray-400 dark:text-slate-500"
                  }
                `}>
                  {count}
                </span>
              </button>
            );
          })}
        </div>
        <div className="flex items-center gap-1.5">
          {/* Date range filters */}
          {(["all", "today", "week", "half-month"] as DateRange[]).map((range) => {
            const labelKey = range === "all" ? "dateRange.all" : range === "today" ? "dateRange.today" : range === "week" ? "dateRange.week" : "dateRange.halfMonth";
            const label = t(labelKey);
            const isActive = dateRange === range;
            return (
              <button
                key={range}
                onClick={() => setDateRange(isActive && range !== "all" ? "all" : range)}
                className={`text-[11px] px-2.5 py-1 rounded-md border transition-all
                  ${isActive
                    ? "text-white bg-orange-500 border-orange-500"
                    : "text-gray-400 dark:text-slate-500 border-gray-200/60 dark:border-white/[0.08] bg-white/60 dark:bg-white/[0.04] hover:border-orange-300 hover:text-orange-500"
                  }`}
              >
                {label}
              </button>
            );
          })}

          {/* Separator */}
          <div className="w-px h-4 bg-gray-200/60 dark:bg-white/[0.08] mx-0.5" />

          <div ref={importPanelRef} className="relative">
            <button
              onClick={openImportPanel}
              disabled={isImportBusy}
              className={`text-[11px] px-2.5 py-1 rounded-md border transition-all flex items-center gap-1 disabled:opacity-60
                ${importStatus === "done"
                  ? "text-green-600 border-green-300 bg-green-50"
                  : importStatus === "error"
                  ? "text-red-500 border-red-200 bg-red-50 dark:bg-red-500/10"
                  : isImportBusy
                  ? "text-orange-500 border-orange-300 bg-orange-50 animate-pulse"
                  : "text-gray-400 dark:text-slate-500 border-gray-200/60 dark:border-white/[0.08] bg-white/60 dark:bg-white/[0.04] hover:border-orange-300 hover:text-orange-500"
                }`}
            >
              <Import size={13} />
              {isImportBusy ? t("import.importing") : t("import.button")}
            </button>
            {renderImportPanel("right")}
          </div>

          {/* Export current view */}
          <button
            onClick={async () => {
              if (exportStatus === "idle") {
                // First click: show confirm
                setExportStatus("confirm");
                confirmTimer.current = setTimeout(() => setExportStatus("idle"), 3000);
                return;
              }
              if (exportStatus === "confirm") {
                // Second click: do export
                if (confirmTimer.current) clearTimeout(confirmTimer.current);
                setExportStatus("exporting");
                try {
                  if (dateRange === "all") {
                    await exportAllSingle();
                  } else {
                    const now = new Date();
                    const end = now.toISOString().slice(0, 10);
                    const start = new Date();
                    if (dateRange === "today") start.setHours(0, 0, 0, 0);
                    else if (dateRange === "week") start.setDate(now.getDate() - 7);
                    else if (dateRange === "half-month") start.setDate(now.getDate() - 15);
                    await exportRangeSingle(start.toISOString().slice(0, 10), end);
                  }
                  setExportStatus("done");
                  setTimeout(() => setExportStatus("idle"), 3000);
                } catch (e) { console.error(e); setExportStatus("idle"); }
              }
            }}
            disabled={exportStatus === "exporting"}
            className={`text-[11px] px-2.5 py-1 rounded-md border transition-all flex items-center gap-1
              ${exportStatus === "confirm"
                ? "text-orange-600 border-orange-400 bg-orange-100 dark:bg-orange-500/20"
                : exportStatus === "done"
                ? "text-green-600 border-green-300 bg-green-50"
                : exportStatus === "exporting"
                ? "text-orange-500 border-orange-300 bg-orange-50 animate-pulse"
                : "text-gray-400 dark:text-slate-500 border-gray-200/60 dark:border-white/[0.08] bg-white/60 dark:bg-white/[0.04] hover:border-orange-300 hover:text-orange-500"
              }`}
          >
            {exportStatus === "confirm" ? t("export.confirm") : exportStatus === "exporting" ? t("export.exporting") : exportStatus === "done" ? `✓ ${t("export.done")}` : `↗ ${t("export.button")}`}
          </button>

          {/* Capture status */}
          <div className="flex items-center gap-1 text-[11px] text-gray-400 dark:text-slate-500 ml-1">
            <span className={`w-1.5 h-1.5 rounded-full ${captureEnabled ? "bg-green-400" : "bg-gray-300 dark:bg-slate-600"}`} />
            {captureEnabled ? t("capture.active") : t("capture.paused")}
          </div>
        </div>
      </div>

      {importStatus !== "idle" && importMessage && (
        <div className={`px-1 text-xs ${importStatus === "error" ? "text-red-500" : "text-stone-400 dark:text-stone-500"}`}>
          {importMessage}
        </div>
      )}

      {/* Content cards */}
      {filteredContents.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-16 text-center">
          <span className="text-3xl mb-3">🔍</span>
          <p className="text-sm text-gray-500 dark:text-slate-400">
            {t("emptyFilter", { type: t(FILTER_TABS.find((tab) => tab.value === filter)?.labelKey ?? "filter.all") })}
          </p>
        </div>
      ) : (
        <div className="space-y-2.5">
          {filteredContents.map((content) => (
            <ContentCard
              key={content.id}
              content={content}
              isHighlighted={highlightedIds.includes(content.id)}
              ref={(el) => { cardRefs.current[content.id] = el; }}
            />
          ))}
          {hasMore && isLoadingMore && (
            <div className="flex justify-center py-4">
              <span className="text-xs text-gray-400 dark:text-slate-500 animate-pulse">
                {t("loading", "加载中...")}
              </span>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
