import { useState, useRef, useEffect, forwardRef } from "react";
import { createPortal } from "react-dom";
import { motion, AnimatePresence } from "framer-motion";
import { convertFileSrc } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-shell";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import type { CapturedContent } from "../../types/content";
import { deleteContent, retryUrlFetch } from "../../services/storageService";
import { compileContentToWiki, getContentWikiPages } from "../../services/wikiService";

import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useContentStore } from "../../stores/contentStore";
import { useDataHubStore } from "../../stores/dataHubStore";
import { ImagePreview } from "./ImagePreview";
import type { WikiPage } from "../../types/wiki";
import { openContentFile } from "../../services/dataHubService";

interface ContentCardProps {
  content: CapturedContent;
  isHighlighted?: boolean;
  selectionMode?: boolean;
  selected?: boolean;
  onToggleSelect?: () => void;
}

function formatRelativeTime(dateStr: string, t: TFunction): string {
  const date = new Date(dateStr);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffSec = Math.floor(diffMs / 1000);
  const diffMin = Math.floor(diffSec / 60);
  const diffHour = Math.floor(diffMin / 60);
  const diffDay = Math.floor(diffHour / 24);

  if (diffSec < 60) return t("card.justNow");
  if (diffMin < 60) return t("card.minutesAgo", { count: diffMin });
  if (diffHour < 24) return t("card.hoursAgo", { count: diffHour });
  if (diffDay < 7) return t("card.daysAgo", { count: diffDay });
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export const ContentCard = forwardRef<HTMLDivElement, ContentCardProps>(
  function ContentCard({
    content,
    isHighlighted = false,
    selectionMode = false,
    selected = false,
    onToggleSelect,
  }, ref) {
  const { t } = useTranslation("content");
  const removeContent = useContentStore((s) => s.removeContent);
  const removeFromDataHub = useDataHubStore((s) => s.removeContent);
  const [previewOpen, setPreviewOpen] = useState(false);
  const [textExpanded, setTextExpanded] = useState(false);
  const [copied, setCopied] = useState(false);
  const [deleteState, setDeleteState] = useState<"idle" | "confirm" | "deleting">("idle");
  const [ocrState] = useState<"idle" | "running" | "done">("idle");
  const [ocrText] = useState<string | null>(null);
  const [wikiState, setWikiState] = useState<"idle" | "compiling" | "done">("idle");
  const [openingSaved, setOpeningSaved] = useState(false);
  const [linkedWikiPages, setLinkedWikiPages] = useState<WikiPage[]>([]);

  const deleteTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Load linked wiki pages on mount
  useEffect(() => {
    getContentWikiPages(content.id).then(setLinkedWikiPages).catch(() => {});
  }, [content.id]);

  const handleWikiCompile = async () => {
    if (wikiState === "compiling") return;
    setWikiState("compiling");
    try {
      await compileContentToWiki(content.id);
      setWikiState("done");
      // Refresh linked pages
      const pages = await getContentWikiPages(content.id);
      setLinkedWikiPages(pages);
      setTimeout(() => setWikiState("idle"), 2000);
    } catch (e) {
      console.error("Wiki compile failed:", e);
      setWikiState("idle");
    }
  };

  const handleDelete = async () => {
    if (deleteState === "idle") {
      setDeleteState("confirm");
      // Auto-reset after 3 seconds if user doesn't confirm
      deleteTimerRef.current = setTimeout(() => setDeleteState("idle"), 3000);
      return;
    }
    if (deleteState === "confirm") {
      if (deleteTimerRef.current) clearTimeout(deleteTimerRef.current);
      setDeleteState("deleting");
      try {
        await deleteContent(content.id);
        removeContent(content.id);
        removeFromDataHub(content.id);
      } catch (e) {
        console.error("Failed to delete:", e);
        setDeleteState("idle");
      }
    }
  };

  const handleCopy = async () => {
    const textToCopy = content.clean_content || content.raw_text;
    if (!textToCopy) return;
    try {
      await navigator.clipboard.writeText(textToCopy);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch (e) {
      console.error("Failed to copy:", e);
    }
  };

  const handleOpenSaved = async () => {
    if (openingSaved) return;
    setOpeningSaved(true);
    try {
      await openContentFile(content.id);
    } catch (e) {
      console.error("Failed to open saved content:", e);
    } finally {
      setOpeningSaved(false);
    }
  };

  const typeConfig = {
    image: { icon: "🖼️", label: t("content:filter.image"), accent: "bg-amber-500/10 dark:bg-amber-500/20" },
    url: { icon: "🔗", label: t("content:filter.url"), accent: "bg-orange-500/10 dark:bg-orange-500/20" },
    text: { icon: "📝", label: t("content:filter.text"), accent: "bg-orange-500/10 dark:bg-orange-500/20" },
    mixed: { icon: "📎", label: t("common:contentType.mixed"), accent: "bg-gray-500/10 dark:bg-gray-500/20" },
  };

  const { icon: typeIcon } = typeConfig[content.content_type] || typeConfig.text;
  const timeStr = formatRelativeTime(content.captured_at, t);

  const [retrying, setRetrying] = useState(false);

  const handleRetry = async () => {
    setRetrying(true);
    try {
      await retryUrlFetch(content.id);
    } catch (e) {
      console.error("Retry failed:", e);
    }
    // Don't reset retrying — the list will reload when content:url-fetched fires
  };

  // URL content states
  const isUrlContent = content.content_type === "url";
  const hasSourceUrl = isUrlContent && !!content.source_url;
  // Check if URL fetch failed (raw_text starts with [读取失败])
  const isFailedUrl = hasSourceUrl && content.raw_text?.startsWith("[读取失败]");
  // Extract the actual error reason from the failure payload. Backend writes
  // raw_text as: "[读取失败] {error}\n\n原始链接: {url}" — we peel off the
  // prefix and stop at the double newline to recover {error}. Showing this
  // to the user lets them distinguish "yt-dlp not found" from "429 rate
  // limited" from "needs sign-in" without digging into logs.
  const failureReason = isFailedUrl
    ? content.raw_text
        ?.slice("[读取失败] ".length)
        .split("\n\n")[0]
        .trim()
    : null;
  // raw_text 不等于 source_url 就说明已完成读取（可能是正文、标题、或视频号标记）
  const isFetchedUrl = hasSourceUrl && !isFailedUrl && content.raw_text &&
    content.raw_text.trim() !== content.source_url?.trim();
  const isLoadingUrl = hasSourceUrl && !isFetchedUrl && !isFailedUrl;

  // Only show "AI 分析中" for content captured in the last 2 minutes
  const isRecent = Date.now() - new Date(content.captured_at).getTime() < 2 * 60 * 1000;

  const imageSrc =
    content.content_type === "image"
      ? content.thumbnail_path
        ? convertFileSrc(content.thumbnail_path)
        : content.image_path
          ? convertFileSrc(content.image_path)
          : null
      : null;

  const fullImageSrc =
    content.content_type === "image" && content.image_path
      ? convertFileSrc(content.image_path)
      : null;

  return (
    <>
      <div
        ref={ref}
        className={`
        group rounded-2xl transition-all duration-300 relative
        ${isHighlighted
          ? "ring-2 ring-orange-300/60 dark:ring-orange-500/30 animate-highlight-fade"
          : selected
            ? "ring-2 ring-orange-400/70 dark:ring-orange-500/40"
          : deleteState !== "idle"
            ? "ring-1 ring-red-200/80 dark:ring-red-500/30"
            : "hover:translate-y-[-1px] hover:shadow-[0_12px_40px_rgba(249,115,22,0.12)] dark:hover:shadow-[0_12px_40px_rgba(0,0,0,0.3)]"
        }
        glass
      `}>
        {selectionMode && onToggleSelect && (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onToggleSelect();
            }}
            className={`absolute top-3 right-3 z-10 w-6 h-6 rounded-full border transition-all flex items-center justify-center
              ${selected
                ? "bg-orange-500 border-orange-500 text-white shadow-sm"
                : "bg-white/90 dark:bg-slate-900/90 border-gray-200/80 dark:border-white/10 text-gray-400 dark:text-slate-500"
              }`}
            aria-label={selected ? t("card.unselect") : t("card.select")}
          >
            {selected ? "✓" : ""}
          </button>
        )}
        {/* Main content area */}
        <div className="px-5 pt-4 pb-3">
          <div className="flex gap-3.5 items-start">
            {/* Type icon */}
            <div className={`w-8 h-8 rounded-lg flex items-center justify-center flex-shrink-0 mt-0.5 ${
              content.content_type === "url" ? "bg-orange-50 dark:bg-orange-500/10" :
              content.content_type === "image" ? "bg-green-50 dark:bg-green-500/10" :
              "bg-blue-50 dark:bg-blue-500/10"
            }`}>
              <span className="text-[15px]">{typeIcon}</span>
            </div>

            {/* Content body */}
            <div className="flex-1 min-w-0">
          {/* Clickable content area — tags + summary, click anywhere to expand */}
          <div
            className="cursor-pointer"
            onClick={() => {
              if (content.raw_text || ocrText || (isUrlContent && isFetchedUrl)) {
                setTextExpanded(true);
              }
            }}
          >
            {/* Tags */}
            <TagChips
              tags={content.tags}
              analyzing={!content.tags && !!(content.raw_text || ocrText) && (content.raw_text?.length ?? 0) >= 6 && isRecent}
            />

            {/* Image content: thumbnail + summary side by side */}
            {imageSrc && (
              <div className="flex gap-3.5 items-start mb-1">
                <div
                  className="cursor-pointer group/img flex-shrink-0"
                  onClick={(e) => { e.stopPropagation(); setPreviewOpen(true); }}
                >
                  <img
                    src={imageSrc}
                    alt="Captured"
                    className="w-20 h-20 rounded-[10px] border border-gray-200/60 dark:border-white/10
                               group-hover/img:border-orange-300/60 dark:group-hover/img:border-orange-500/40
                               group-hover/img:shadow-md transition-all object-cover"
                    loading="lazy"
                  />
                </div>
                <div className="flex-1 min-w-0 flex flex-col justify-center">
                  {content.summary ? (
                    <p className="text-sm text-gray-700 dark:text-gray-200 leading-relaxed">
                      {content.summary}
                    </p>
                  ) : content.raw_text || ocrText ? (
                    <p className="text-sm text-gray-500 dark:text-gray-400 leading-relaxed line-clamp-2">
                      {(ocrText || content.raw_text || "").slice(0, 80)}...
                    </p>
                  ) : null}
                  {(content.raw_text || ocrText) && (
                    <span className="text-[11px] text-gray-400 dark:text-slate-500 mt-1">
                      {t("card.ocrRecognized", { count: (ocrText || content.raw_text || "").length })}
                    </span>
                  )}
                </div>
              </div>
            )}

            {/* OCR loading indicator */}
            {content.content_type === "image" && !imageSrc && !content.raw_text && !ocrText && ocrState === "running" && (
              <div className="mb-2 flex items-center gap-1.5 text-xs text-amber-500 dark:text-amber-400">
                <svg className="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                </svg>
                {t("card.ocrRunning")}
              </div>
            )}

            {/* URL content: fetched */}
            {isUrlContent && isFetchedUrl && (
              <div>
                {content.summary ? (
                  <p className="text-sm text-gray-700 dark:text-gray-200 leading-relaxed">
                    {content.summary}
                  </p>
                ) : (
                  <p className="text-sm text-gray-500 dark:text-gray-400 leading-relaxed line-clamp-2">
                    {content.raw_text?.slice(0, 80)}{(content.raw_text?.length ?? 0) > 80 ? "..." : ""}
                  </p>
                )}
              </div>
            )}

            {/* Text content (non-URL, non-image) */}
            {!isUrlContent && content.content_type !== "image" && content.raw_text && (
              <div>
                {content.summary ? (
                  <p className="text-sm text-gray-700 dark:text-gray-200 leading-relaxed" style={{ overflowWrap: "anywhere", wordBreak: "break-word" }}>
                    {content.summary}
                  </p>
                ) : (
                  <p className="text-sm text-gray-500 dark:text-gray-400 leading-relaxed line-clamp-2" style={{ overflowWrap: "anywhere", wordBreak: "break-word" }}>
                    {content.raw_text.slice(0, 80)}{content.raw_text.length > 80 ? "..." : ""}
                  </p>
                )}
              </div>
            )}

            {/* No content fallback */}
            {!imageSrc && !content.raw_text && !isUrlContent && (
              <p className="text-sm text-gray-400 dark:text-slate-500 italic">{t("card.noContent")}</p>
            )}
          </div>

          {/* URL content: loading — outside clickable area */}
          {isUrlContent && isLoadingUrl && (
            <div className="flex items-center gap-2">
              <p className="text-sm text-orange-500 dark:text-orange-400 truncate flex-1">
                {content.source_url}
              </p>
              <span className="flex items-center gap-1.5 text-xs text-gray-400 dark:text-slate-500 flex-shrink-0">
                <svg className="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                </svg>
                {t("card.fetching")}
              </span>
            </div>
          )}

          {/* URL content: failed */}
          {isUrlContent && isFailedUrl && (
            <div>
              <div className="flex items-center gap-2 mb-1.5">
                <span className="text-xs text-red-500 dark:text-red-400 font-medium">{t("card.fetchFailed")}</span>
                <button
                  onClick={handleRetry}
                  disabled={retrying}
                  className="inline-flex items-center gap-1 text-xs text-orange-500 dark:text-orange-400
                             hover:text-orange-600 dark:hover:text-orange-300
                             disabled:opacity-50 transition-colors"
                >
                  <svg className={`w-3 h-3 ${retrying ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                  </svg>
                  {retrying ? t("card.retrying") : t("card.retry")}
                </button>
              </div>
              {failureReason && (
                <p className="text-xs text-red-500/80 dark:text-red-400/80 mb-1 leading-relaxed">
                  {failureReason}
                </p>
              )}
              <p className="text-sm text-orange-500 dark:text-orange-400 truncate">
                {content.source_url}
              </p>
            </div>
          )}


          {/* User note */}
          {content.user_note && (
            <div className="mt-2 flex items-start gap-1.5 px-2.5 py-1.5 rounded-lg
                            bg-orange-500/[0.06] dark:bg-orange-500/[0.08]
                            border border-orange-200/40 dark:border-orange-500/15">
              <span className="text-xs leading-none mt-0.5">💬</span>
              <span className="text-xs text-orange-600 dark:text-orange-300 leading-relaxed">
                {content.user_note}
              </span>
            </div>
          )}

          {/* Footer: meta + actions */}
          <div className="flex items-center justify-between mt-3.5 pt-2.5 border-t"
               style={{ borderColor: "var(--color-border-light, rgba(0,0,0,0.04))" }}>
            <div className="flex items-center gap-2 text-[11px] text-gray-400 dark:text-slate-500">
              <span className={`w-[6px] h-[6px] rounded-full flex-shrink-0 ${
                (() => {
                  const ageMs = Date.now() - new Date(content.captured_at).getTime();
                  const hours = ageMs / (1000 * 60 * 60);
                  if (hours < 1) return "bg-green-500";       // < 1小时：鲜绿
                  if (hours < 6) return "bg-green-400";       // < 6小时：浅绿
                  if (hours < 24) return "bg-yellow-400";     // < 1天：黄色
                  if (hours < 72) return "bg-orange-300";     // < 3天：浅橙
                  if (hours < 168) return "bg-stone-400";     // < 1周：灰色
                  return "bg-stone-300";                      // > 1周：浅灰
                })()
              }`} />
              <span>{timeStr}</span>
              <span className="text-gray-300/80 dark:text-slate-600">/</span>
              <span>{content.source_app}</span>
            </div>

            <div className="flex items-center gap-1">
              {(content.image_path || content.raw_text || content.clean_content) && (
                <button
                  onClick={handleOpenSaved}
                  disabled={openingSaved}
                  className="px-2 py-1 rounded-md text-[11px] text-gray-400 dark:text-slate-500
                             hover:text-orange-600 dark:hover:text-orange-400
                             hover:bg-orange-500/10 dark:hover:bg-orange-500/15 transition-all
                             disabled:opacity-60"
                >
                  {openingSaved ? "..." : t("card.openSaved")}
                </button>
              )}
              {hasSourceUrl && (
                <button
                  onClick={() => content.source_url && open(content.source_url)}
                  className="px-2 py-1 rounded-md text-[11px] text-gray-400 dark:text-slate-500
                             hover:text-orange-600 dark:hover:text-orange-400
                             hover:bg-orange-500/10 dark:hover:bg-orange-500/15 transition-all"
                >
                  {t("card.openLink")}
                </button>
              )}
              {content.raw_text && (
                <button
                  onClick={handleCopy}
                  className={`px-2 py-1 rounded-md text-[11px] transition-all
                    ${copied
                      ? "text-green-600 dark:text-green-400"
                      : "text-gray-400 dark:text-slate-500 hover:text-orange-600 dark:hover:text-orange-400 hover:bg-orange-500/10 dark:hover:bg-orange-500/15"
                    }`}
                >
                  {copied ? t("card.copied") : t("card.copy")}
                </button>
              )}
              {(content.raw_text || content.user_note || content.source_url) && (
                <button
                  onClick={handleWikiCompile}
                  disabled={wikiState === "compiling"}
                  className={`px-2 py-1 rounded-md text-[11px] transition-all
                    ${wikiState === "done"
                      ? "text-green-600 dark:text-green-400"
                      : wikiState === "compiling"
                      ? "text-orange-400 dark:text-orange-500 opacity-60"
                      : "text-gray-400 dark:text-slate-500 hover:text-orange-600 dark:hover:text-orange-400 hover:bg-orange-500/10 dark:hover:bg-orange-500/15"
                    }`}
                >
                  {wikiState === "done" ? t("card.compiledToWiki") : wikiState === "compiling" ? t("card.compiling") : t("card.addToWiki")}
                </button>
              )}
              <button
                onClick={handleDelete}
                disabled={deleteState === "deleting"}
                className={`px-2 py-1 rounded-md text-[11px] transition-all
                  ${deleteState === "confirm"
                    ? "text-white bg-red-500 hover:bg-red-600 rounded-md"
                    : deleteState === "deleting"
                    ? "text-white bg-red-400 opacity-60 rounded-md"
                    : "text-gray-400 dark:text-slate-500 hover:text-red-500 dark:hover:text-red-400 hover:bg-red-500/10 dark:hover:bg-red-500/15"
                  }`}
              >
                {deleteState === "confirm" ? t("card.deleteConfirmBtn") : deleteState === "deleting" ? "..." : t("card.delete")}
              </button>
            </div>
          </div>
            {/* Wiki linked pages */}
            {linkedWikiPages.length > 0 && (
              <div className="flex items-center gap-1.5 mt-2 pt-2 border-t" style={{ borderColor: "var(--color-border, #E7E5E4)" }}>
                <span style={{ fontSize: 10, color: "var(--color-text-muted, #A8A29E)" }}>{t("card.linkedKnowledge")}</span>
                {linkedWikiPages.slice(0, 3).map((wp) => (
                  <button
                    key={wp.id}
                    onClick={(e) => {
                      e.stopPropagation();
                      window.dispatchEvent(new CustomEvent("navigate-to-wiki-page", { detail: { pageId: wp.id } }));
                    }}
                    className="px-2 py-0.5 rounded-full text-[10px] font-medium transition-colors
                               hover:bg-orange-100 dark:hover:bg-orange-500/15"
                    style={{
                      color: "#F97316",
                      backgroundColor: "#F9731610",
                      border: "1px solid #F9731625",
                    }}
                  >
                    {wp.title}
                  </button>
                ))}
              </div>
            )}
            </div>{/* close content body */}
          </div>{/* close flex row */}
        </div>

      </div>

      {previewOpen && fullImageSrc && (
        <ImagePreview
          src={fullImageSrc}
          onClose={() => setPreviewOpen(false)}
        />
      )}

      {/* Full text overlay — portal to body to escape overflow-hidden */}
      {createPortal(
        <AnimatePresence>
          {textExpanded && (content.raw_text || ocrText || fullImageSrc) && (
            <FullTextOverlay
              content={content}
              copied={copied}
              onCopy={handleCopy}
              onClose={() => setTextExpanded(false)}
              imageSrc={fullImageSrc}
              ocrText={ocrText}
            />
          )}
        </AnimatePresence>,
        document.body
      )}
    </>
  );
});

/* ================================================================
   AUTO-FORMAT — turn text with lightweight Markdown into styled elements
   ================================================================ */
function FormattedText({ text }: { text: string }) {
  const paragraphs = text.split(/\n{2,}/);

  return (
    <div className="space-y-4" style={{ overflowWrap: "anywhere", wordBreak: "break-word" }}>
      {paragraphs.map((para, i) => {
        const trimmed = para.trim();
        if (!trimmed) return null;

        // ── Headings: # Title / ## Title / ### Title ──
        const headingMatch = trimmed.match(/^(#{1,6})\s+(.+)/);
        if (headingMatch) {
          const level = headingMatch[1].length;
          const headingText = headingMatch[2];
          const cls = level === 1
            ? "text-lg font-bold text-gray-900 dark:text-gray-100 mt-2"
            : level === 2
            ? "text-base font-semibold text-gray-800 dark:text-gray-200 mt-1"
            : "text-sm font-semibold text-gray-700 dark:text-gray-300";
          return <h3 key={i} className={cls}>{headingText}</h3>;
        }

        const lines = trimmed.split("\n");

        // ── Blockquote: lines starting with > ──
        const isBlockquote = lines.every(l => l.trim().startsWith(">") || !l.trim());
        if (isBlockquote && lines.some(l => l.trim().startsWith(">"))) {
          return (
            <blockquote
              key={i}
              className="border-l-2 border-orange-300 dark:border-orange-600 pl-3 py-1 text-[14px] text-gray-600 dark:text-gray-400 italic leading-relaxed"
            >
              {lines.filter(l => l.trim()).map((line, j) => (
                <span key={j}>
                  {j > 0 && <br />}
                  {line.replace(/^\s*>\s?/, "")}
                </span>
              ))}
            </blockquote>
          );
        }

        // ── List: lines starting with - / • / * / 1. / 1) ──
        const listLines = lines.filter(l => l.trim());
        const listCount = listLines.filter(l => /^\s*[-•·*+]\s|^\s*\d+[.)、]\s/.test(l)).length;
        const isList = listLines.length > 1 && listCount >= listLines.length * 0.6;
        if (isList) {
          // Detect ordered vs unordered
          const isOrdered = listLines.some(l => /^\s*\d+[.)、]\s/.test(l));
          return (
            <ul key={i} className="space-y-1.5 pl-1">
              {listLines.map((line, j) => {
                const content = line.replace(/^\s*[-•·*+]\s*|^\s*\d+[.)、]\s*/, "");
                const marker = isOrdered ? `${j + 1}.` : "•";
                return (
                  <li key={j} className="flex gap-2 text-[14px] text-gray-700 dark:text-gray-200 leading-relaxed">
                    <span className="text-orange-400 dark:text-orange-500 flex-shrink-0 mt-0.5 text-xs min-w-[16px]">{marker}</span>
                    <span>{content || line}</span>
                  </li>
                );
              })}
            </ul>
          );
        }

        // ── Short standalone line → sub-heading ──
        if (trimmed.length < 40 && !trimmed.endsWith("。") && !trimmed.endsWith("，") && !trimmed.endsWith(".") && !trimmed.endsWith(",") && !trimmed.includes("\n")) {
          return (
            <h4 key={i} className="text-[15px] font-semibold text-gray-800 dark:text-gray-200 mt-1">
              {trimmed}
            </h4>
          );
        }

        // ── Regular paragraph (may contain inline list items) ──
        return (
          <p key={i} className="text-[14px] text-gray-700 dark:text-gray-200 leading-[1.85]">
            {lines.map((line, j) => {
              // Inline list item within a paragraph
              const listMatch = line.match(/^\s*[-•·*+]\s+(.*)/);
              if (listMatch) {
                return (
                  <span key={j} className="flex gap-2 mt-1">
                    <span className="text-orange-400 dark:text-orange-500 flex-shrink-0">•</span>
                    <span>{listMatch[1]}</span>
                  </span>
                );
              }
              return (
                <span key={j}>
                  {j > 0 && <br />}
                  {line}
                </span>
              );
            })}
          </p>
        );
      })}
    </div>
  );
}

/* ================================================================
   FULL TEXT OVERLAY
   ================================================================ */

function AnalyzingChip() {
  const { t } = useTranslation("content");
  return (
    <div className="flex flex-wrap gap-1.5 mb-2">
      <span className="text-[11px] font-medium px-2.5 py-0.5 rounded-full animate-pulse
                        bg-orange-50 dark:bg-orange-500/10 text-orange-400 dark:text-orange-500">
        {t("card.aiAnalyzing")}
      </span>
    </div>
  );
}

function TagChips({ tags, analyzing }: { tags?: string; analyzing?: boolean }) {
  if (!tags && analyzing) return <AnalyzingChip />;
  if (!tags) return null;
  const tagList = tags.split(",").map((t) => t.trim()).filter(Boolean);
  if (tagList.length === 0) return null;
  return (
    <div className="flex flex-wrap gap-1.5 mb-3">
      {tagList.map((tag, i) => (
        <span
          key={i}
          className="rounded-full px-2.5 py-0.5"
          style={{
            fontSize: 12,
            color: "#F97316",
            backgroundColor: "#F9731610",
            border: "1px solid #F9731625",
          }}
        >
          {tag}
        </span>
      ))}
    </div>
  );
}

export function FullTextOverlay({
  content,
  copied,
  onCopy,
  onClose,
  imageSrc,
  ocrText,
}: {
  content: CapturedContent;
  copied: boolean;
  onCopy: () => void;
  onClose: () => void;
  imageSrc?: string | null;
  ocrText?: string | null;
}) {
  const { t } = useTranslation("content");
  const isImage = content.content_type === "image";
  const isUrl = content.content_type === "url";
  // For images, prefer ocrText over content.raw_text
  // Prefer clean_content for URL articles, fallback to raw_text
  const displayText = isImage
    ? (ocrText || content.raw_text)
    : (content.clean_content || content.raw_text);
  const hasCleanContent = !!content.clean_content;
  // Lock background scroll
  useEffect(() => {
    document.body.style.overflow = "hidden";
    return () => { document.body.style.overflow = ""; };
  }, []);

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.2 }}
      className="fixed inset-0 z-50 flex items-center justify-center p-6"
      onClick={onClose}
    >
      {/* Backdrop */}
      <div className="absolute inset-0 bg-black/50 backdrop-blur-md" />
      {/* Panel */}
      <motion.div
        initial={{ opacity: 0, scale: 0.95, y: 10 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.95, y: 10 }}
        transition={{ duration: 0.2, ease: "easeOut" }}
        layout
        className={`relative rounded-2xl overflow-hidden glass-elevated flex flex-col
                    w-full max-w-2xl max-h-[85vh]`}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Top accent line */}
        <div className="absolute inset-x-0 top-0 h-[2px] z-10"
          style={{ background: "linear-gradient(90deg, transparent, rgba(249,115,22,0.4) 30%, rgba(249,115,22,0.5) 50%, rgba(249,115,22,0.4) 70%, transparent)" }}
        />
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 flex-shrink-0">
          <div className="flex items-center gap-3 min-w-0 flex-1">
            <div className={`w-9 h-9 rounded-xl bg-gradient-to-br flex items-center justify-center flex-shrink-0 border
              ${isImage
                ? "from-amber-500/15 to-orange-500/15 dark:from-amber-500/20 dark:to-orange-500/20 border-amber-200/30 dark:border-amber-500/15"
                : isUrl
                  ? "from-orange-500/15 to-orange-500/15 dark:from-orange-500/20 dark:to-orange-500/20 border-orange-200/30 dark:border-orange-500/15"
                  : "from-orange-500/15 to-orange-500/15 dark:from-orange-500/20 dark:to-orange-500/20 border-orange-200/30 dark:border-orange-500/15"
              }`}>
              <span className="text-base">{isImage ? "🖼️" : isUrl ? "🔗" : "📝"}</span>
            </div>
            <div className="min-w-0">
              <div className="text-[13px] font-semibold text-gray-800 dark:text-gray-100 truncate">
                {content.raw_text?.split("\n")[0]?.slice(0, 60) || (isImage ? t("card.imageContent") : t("card.contentDetail"))}
              </div>
              <div className="text-[11px] text-gray-400 dark:text-slate-500 truncate mt-0.5">
                {content.source_url || `${content.source_app} · ${content.content_type}`}
              </div>
            </div>
          </div>
          <div className="flex items-center gap-1.5 flex-shrink-0 ml-3">
            {content.source_url && (
              <button
                onClick={() => open(content.source_url!)}
                className="h-8 px-3 rounded-xl text-xs font-medium transition-all
                           text-gray-500 dark:text-slate-400 hover:text-orange-600 dark:hover:text-orange-400
                           hover:bg-orange-500/8 dark:hover:bg-orange-500/10
                           flex items-center gap-1.5"
              >
                <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.8}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M13.5 6H5.25A2.25 2.25 0 003 8.25v10.5A2.25 2.25 0 005.25 21h10.5A2.25 2.25 0 0018 18.75V10.5m-10.5 6L21 3m0 0h-5.25M21 3v5.25" />
                </svg>
                {t("card.originalArticle")}
              </button>
            )}
            <button
              onClick={onCopy}
              className={`h-8 px-3 rounded-xl text-xs font-medium transition-all flex items-center gap-1.5
                ${copied
                  ? "bg-green-500/10 text-green-600 dark:text-green-400"
                  : "text-gray-500 dark:text-slate-400 hover:text-orange-600 dark:hover:text-orange-400 hover:bg-orange-500/8 dark:hover:bg-orange-500/10"
                }`}
            >
              {copied ? (
                <>
                  <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
                  </svg>
                  {t("card.copied")}
                </>
              ) : (
                <>
                  <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={1.8}>
                    <path strokeLinecap="round" strokeLinejoin="round" d="M15.666 3.888A2.25 2.25 0 0013.5 2.25h-3c-1.03 0-1.9.693-2.166 1.638m7.332 0c.055.194.084.4.084.612v0a.75.75 0 01-.75.75H9.75a.75.75 0 01-.75-.75v0c0-.212.03-.418.084-.612m7.332 0c.646.049 1.288.11 1.927.184 1.1.128 1.907 1.077 1.907 2.185V19.5a2.25 2.25 0 01-2.25 2.25H6.75A2.25 2.25 0 014.5 19.5V6.257c0-1.108.806-2.057 1.907-2.185a48.208 48.208 0 011.927-.184" />
                  </svg>
                  {t("card.copy")}
                </>
              )}
            </button>
            <button
              onClick={onClose}
              className="w-8 h-8 rounded-xl flex items-center justify-center
                         text-gray-400 dark:text-slate-500 hover:text-gray-600 dark:hover:text-slate-300
                         hover:bg-gray-500/8 dark:hover:bg-white/[0.08] transition-all"
            >
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>
        </div>
        {/* Divider */}
        <div className="mx-6 h-[1px] bg-gradient-to-r from-transparent via-gray-200/80 dark:via-white/[0.06] to-transparent flex-shrink-0" />

        {/* Body */}
        <div className="flex-1 min-h-0 flex flex-col">
          <div className="overflow-y-auto w-full">
            <div className="px-6 py-5">
              {/* Image display */}
              {isImage && imageSrc && (
                <div className="mb-4 flex justify-center">
                  <img
                    src={imageSrc}
                    alt="Captured"
                    className="max-w-full max-h-[50vh] rounded-xl border border-white/50 dark:border-white/10 object-contain"
                  />
                </div>
              )}
              {/* Digest — paragraph summary */}
              {content.digest && (
                <div className="mb-4 rounded-xl p-4" style={{
                  backgroundColor: "var(--color-surface-raised, #F5F5F0)",
                  border: "1px solid var(--color-border, #E7E5E4)",
                }}>
                  <div className="flex items-center gap-1.5 mb-2">
                    <span className="w-1 h-1 rounded-full" style={{ backgroundColor: "#F97316" }} />
                    <span style={{ fontSize: 11, fontWeight: 600, color: "#F97316" }}>{t("card.aiSummary")}</span>
                  </div>
                  <p style={{ fontSize: 13, lineHeight: 1.8, color: "var(--color-text-secondary, #57534E)" }}>
                    {content.digest}
                  </p>
                </div>
              )}
              {/* Text content — auto-formatted */}
              {displayText && (
                <article className="selection:bg-orange-500/20 dark:selection:bg-orange-500/30 overflow-hidden">
                  {isImage && (
                    <div className="flex items-center gap-1.5 mb-3">
                      <span className="text-[11px] text-amber-600 dark:text-amber-400 font-medium px-2 py-0.5 rounded-md bg-amber-500/10">{t("card.ocrText")}</span>
                    </div>
                  )}
                  {hasCleanContent ? (
                    <div className="prose prose-sm dark:prose-invert max-w-none prose-headings:font-semibold prose-p:leading-relaxed prose-a:text-orange-500">
                      <ReactMarkdown remarkPlugins={[remarkGfm]}>{displayText}</ReactMarkdown>
                    </div>
                  ) : (
                    <FormattedText text={displayText} />
                  )}
                </article>
              )}
              {/* No text fallback for images */}
              {isImage && !displayText && (
                <p className="text-sm text-gray-400 dark:text-slate-500 italic text-center">
                  {t("card.noOcrText")}
                </p>
              )}
            </div>
          </div>

        </div>
      </motion.div>
    </motion.div>
  );
}
