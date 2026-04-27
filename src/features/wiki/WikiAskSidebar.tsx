import { useState, useEffect, useRef, useCallback } from "react";
import { Send, X, Plus, Trash2, BookOpen, Loader, ChevronLeft } from "lucide-react";
import { useTranslation } from "react-i18next";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { WikiChatSession, WikiChatMessage } from "../../types/wiki";
import { wikiAsk, getChatSessions, getChatMessages, deleteChatSession, saveMessageAsPage, getWikiPage, getSavedMessageIds } from "../../services/wikiService";

interface WikiAskSidebarProps {
  onClose: () => void;
  onNavigateToPage?: (pageId: string) => void;
}

export function WikiAskSidebar({ onClose, onNavigateToPage }: WikiAskSidebarProps) {
  const { t } = useTranslation("wiki");
  const [sessions, setSessions] = useState<WikiChatSession[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [messages, setMessages] = useState<WikiChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [isAsking, setIsAsking] = useState(false);
  const [savingId, setSavingId] = useState<string | null>(null);
  const [savedIds, setSavedIds] = useState<Set<string>>(new Set());
  const inFlightSaveRef = useRef<Set<string>>(new Set());
  const [view, setView] = useState<"list" | "chat">("list");
  const messagesEndRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    loadSessions();
  }, []);

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  const loadSessions = async () => {
    try {
      const s = await getChatSessions(30);
      setSessions(s);
    } catch (e) {
      console.error("Failed to load sessions:", e);
    }
  };

  const openSession = async (sessionId: string) => {
    setActiveSessionId(sessionId);
    setView("chat");
    try {
      const msgs = await getChatMessages(sessionId);
      setMessages(msgs);
      // Hydrate savedIds from backend — check which assistant messages are already saved
      const asstIds = msgs.filter(m => m.role === "assistant").map(m => m.id);
      if (asstIds.length > 0) {
        const saved = await getSavedMessageIds(asstIds);
        if (saved.length > 0) {
          setSavedIds(new Set(saved));
          saved.forEach(id => inFlightSaveRef.current.add(id));
        }
      }
    } catch (e) {
      console.error("Failed to load messages:", e);
      setMessages([]);
    }
  };

  const startNewSession = () => {
    const newId = crypto.randomUUID();
    setActiveSessionId(newId);
    setMessages([]);
    setView("chat");
  };

  const handleSend = useCallback(async () => {
    if (!input.trim() || isAsking || !activeSessionId) return;
    const question = input.trim();
    setInput("");
    setIsAsking(true);

    // Optimistic: add user message to UI
    const userMsg: WikiChatMessage = {
      id: crypto.randomUUID(),
      session_id: activeSessionId,
      role: "user",
      content: question,
      turn_index: messages.length,
      created_at: new Date().toISOString(),
    };
    setMessages((prev) => [...prev, userMsg]);

    try {
      const result = await wikiAsk(activeSessionId, question);
      const asstMsg: WikiChatMessage = {
        id: result.message_id,
        session_id: activeSessionId,
        role: "assistant",
        content: result.answer,
        pages_used: JSON.stringify(result.pages_used),
        source_mode: result.source_mode,
        turn_index: messages.length + 1,
        created_at: new Date().toISOString(),
      };
      setMessages((prev) => [...prev, asstMsg]);
      loadSessions(); // refresh sidebar list
    } catch (e) {
      const errorMsg: WikiChatMessage = {
        id: crypto.randomUUID(),
        session_id: activeSessionId,
        role: "assistant",
        content: t("ask.requestFailed", { error: String(e) }),
        source_mode: "ai_only",
        turn_index: messages.length + 1,
        created_at: new Date().toISOString(),
      };
      setMessages((prev) => [...prev, errorMsg]);
    }
    setIsAsking(false);
  }, [input, isAsking, activeSessionId, messages, t]);

  const handleSaveAsPage = async (msgId: string) => {
    // Synchronous ref check prevents double-click race
    if (!activeSessionId || inFlightSaveRef.current.has(msgId) || savedIds.has(msgId)) return;
    inFlightSaveRef.current.add(msgId);
    setSavingId(msgId);
    try {
      await saveMessageAsPage(activeSessionId, msgId);
      setSavedIds((prev) => new Set(prev).add(msgId));
    } catch (e) {
      // Backend returns existing page on dedup, so "error" here is a real failure
      inFlightSaveRef.current.delete(msgId);
      alert(t("ask.saveFailed", { error: String(e) }));
    }
    setSavingId(null);
  };

  const handleDeleteSession = async (sessionId: string) => {
    try {
      await deleteChatSession(sessionId);
      setSessions((prev) => prev.filter((s) => s.id !== sessionId));
      if (activeSessionId === sessionId) {
        setActiveSessionId(null);
        setMessages([]);
        setView("list");
      }
    } catch (e) {
      console.error("Failed to delete session:", e);
    }
  };

  // Cache resolved page titles so we don't re-fetch every render
  const [pageTitleCache, setPageTitleCache] = useState<Record<string, string>>({});

  const resolvePageRefs = useCallback(async (pagesUsed: string | undefined): Promise<{ id: string; title: string }[]> => {
    if (!pagesUsed) return [];
    try {
      const parsed = JSON.parse(pagesUsed);
      if (!Array.isArray(parsed)) return [];
      const refs: { id: string; title: string }[] = [];
      for (const item of parsed) {
        if (typeof item === "object" && item && "id" in item && "title" in item) {
          refs.push(item);
        } else if (typeof item === "string" && item) {
          if (pageTitleCache[item]) {
            refs.push({ id: item, title: pageTitleCache[item] });
          } else {
            try {
              const page = await getWikiPage(item);
              const title = page?.title || item.slice(0, 12);
              setPageTitleCache(prev => ({ ...prev, [item]: title }));
              refs.push({ id: item, title });
            } catch {
              refs.push({ id: item, title: item.slice(0, 12) });
            }
          }
        }
      }
      return refs;
    } catch { return []; }
  }, [pageTitleCache]);

  // Resolved refs per message
  const [resolvedRefs, setResolvedRefs] = useState<Record<string, { id: string; title: string }[]>>({});

  useEffect(() => {
    const resolve = async () => {
      const newRefs: Record<string, { id: string; title: string }[]> = {};
      for (const msg of messages) {
        if (msg.role === "assistant" && msg.pages_used) {
          if (!resolvedRefs[msg.id]) {
            newRefs[msg.id] = await resolvePageRefs(msg.pages_used);
          }
        }
      }
      if (Object.keys(newRefs).length > 0) {
        setResolvedRefs(prev => ({ ...prev, ...newRefs }));
      }
    };
    resolve();
  }, [messages]);

  return (
    <div className="flex flex-col h-full" style={{
      backgroundColor: "var(--color-surface, #FFFFFF)",
      borderLeft: "1px solid var(--color-border, #E7E5E4)",
    }}>
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b flex-shrink-0"
        style={{ borderColor: "var(--color-border, #E7E5E4)" }}>
        {view === "chat" ? (
          <button onClick={() => setView("list")} className="flex items-center gap-1 text-sm text-stone-500 hover:text-orange-500 transition-colors">
            <ChevronLeft size={16} />
            <span>{t("ask.chatList")}</span>
          </button>
        ) : (
          <span style={{ fontSize: 14, fontWeight: 600, color: "var(--color-text-primary)" }}>{t("ask.qaTitle")}</span>
        )}
        <div className="flex items-center gap-1">
          <button onClick={startNewSession} className="p-1.5 rounded-lg hover:bg-stone-100 dark:hover:bg-white/[0.08] text-stone-400 hover:text-orange-500 transition-colors" title={t("ask.newChatTooltip")}>
            <Plus size={16} />
          </button>
          <button onClick={onClose} className="p-1.5 rounded-lg hover:bg-stone-100 dark:hover:bg-white/[0.08] text-stone-400 transition-colors">
            <X size={16} />
          </button>
        </div>
      </div>

      {/* Session list view */}
      {view === "list" && (
        <div className="flex-1 overflow-y-auto">
          {sessions.length === 0 ? (
            <div className="text-center py-12">
              <p style={{ fontSize: 13, color: "var(--color-text-muted)" }}>{t("ask.noSessions")}</p>
              <button onClick={startNewSession}
                className="mt-3 px-4 py-2 rounded-lg text-sm font-medium"
                style={{ color: "#F97316", backgroundColor: "#F9731615", border: "1px solid #F9731630" }}>
                {t("ask.startAsking")}
              </button>
            </div>
          ) : (
            <div className="p-2 space-y-1">
              {sessions.map((s) => (
                <div key={s.id} className="flex items-center group">
                  <button
                    onClick={() => openSession(s.id)}
                    className="flex-1 text-left px-3 py-2.5 rounded-lg hover:bg-stone-50 dark:hover:bg-white/[0.04] transition-colors"
                  >
                    <p className="text-sm truncate" style={{ color: "var(--color-text-primary)" }}>
                      {s.title || t("ask.defaultSessionTitle")}
                    </p>
                    <p className="text-[10px] mt-0.5" style={{ color: "var(--color-text-muted)" }}>
                      {s.updated_at?.slice(0, 10)}
                    </p>
                  </button>
                  <button
                    onClick={() => handleDeleteSession(s.id)}
                    className="p-1 rounded opacity-0 group-hover:opacity-100 hover:bg-red-50 dark:hover:bg-red-500/10 text-stone-300 hover:text-red-500 transition-all"
                  >
                    <Trash2 size={13} />
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Chat view */}
      {view === "chat" && (
        <>
          {/* Messages */}
          <div className="flex-1 overflow-y-auto px-4 py-3 space-y-4">
            {messages.length === 0 && !isAsking && (
              <div className="text-center py-8">
                <p style={{ fontSize: 13, color: "var(--color-text-muted)" }}>
                  {t("ask.chatHint")}
                </p>
              </div>
            )}
            {messages.map((msg) => (
              <div key={msg.id}>
                {msg.role === "user" ? (
                  <div className="flex justify-end">
                    <div className="max-w-[85%] px-3 py-2 rounded-xl text-sm"
                      style={{ backgroundColor: "#F97316", color: "white" }}>
                      {msg.content}
                    </div>
                  </div>
                ) : (
                  <div className="max-w-[95%]">
                    {/* Source mode badge */}
                    {msg.source_mode && (
                      <div className="mb-1">
                        <span className="text-[10px] px-1.5 py-0.5 rounded" style={{
                          color: msg.source_mode === "ai_only" ? "var(--color-text-muted)" : "#F97316",
                          backgroundColor: msg.source_mode === "ai_only" ? "var(--color-surface-raised)" : "#F9731610",
                        }}>
                          {msg.source_mode === "knowledge_base" ? t("ask.sourceLabel.knowledge_base") :
                           msg.source_mode === "mixed" ? t("ask.sourceLabel.mixed") : t("ask.sourceLabel.ai_only")}
                        </span>
                      </div>
                    )}
                    {/* Answer content */}
                    <div className="px-3 py-2 rounded-xl text-sm" style={{
                      backgroundColor: "var(--color-surface-raised, #F5F5F0)",
                      border: "1px solid var(--color-border, #E7E5E4)",
                    }}>
                      <article className="prose prose-sm prose-stone dark:prose-invert max-w-none
                        prose-a:text-orange-500 prose-code:text-orange-600 dark:prose-code:text-orange-400
                        prose-code:bg-orange-50 dark:prose-code:bg-orange-500/10
                        prose-code:px-1 prose-code:py-0.5 prose-code:rounded
                        prose-code:before:content-none prose-code:after:content-none"
                        style={{
                          fontSize: 13,
                          lineHeight: 1.7,
                          // Long URLs (GitHub repos, etc.) overflow the
                          // narrow chat sidebar. anywhere lets the
                          // browser break inside any character.
                          overflowWrap: "anywhere",
                          wordBreak: "break-word",
                        }}>
                        <ReactMarkdown remarkPlugins={[remarkGfm]}>{msg.content}</ReactMarkdown>
                      </article>
                    </div>
                    {/* Referenced pages */}
                    {(resolvedRefs[msg.id] || []).length > 0 && (
                      <div className="flex flex-wrap gap-1 mt-1.5">
                        <span className="text-[10px]" style={{ color: "var(--color-text-muted)" }}>{t("ask.references")}</span>
                        {(resolvedRefs[msg.id] || []).map((p, i) => (
                          <button key={p.id || i}
                            onClick={() => p.id && onNavigateToPage?.(p.id)}
                            className="text-[10px] px-1.5 py-0.5 rounded-full hover:bg-orange-100 dark:hover:bg-orange-500/15 transition-colors"
                            style={{ color: "#F97316", backgroundColor: "#F9731610", border: "1px solid #F9731625" }}>
                            {p.title}
                          </button>
                        ))}
                      </div>
                    )}
                    {/* Save button — only for non-ai_only */}
                    {msg.source_mode && msg.source_mode !== "ai_only" && (
                      savedIds.has(msg.id) ? (
                        <span className="flex items-center gap-1 mt-1.5 px-2 py-1 text-[10px] font-medium"
                          style={{ color: "#16A34A" }}>
                          <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
                          {t("ask.savedToKB")}
                        </span>
                      ) : (
                        <button
                          onClick={() => handleSaveAsPage(msg.id)}
                          disabled={savingId === msg.id}
                          className="flex items-center gap-1 mt-1.5 px-2 py-1 rounded text-[10px] font-medium
                            hover:bg-orange-500/10 transition-colors disabled:opacity-40"
                          style={{ color: "#F97316" }}
                        >
                          {savingId === msg.id ? <Loader size={10} className="animate-spin" /> : <BookOpen size={10} />}
                          {savingId === msg.id ? t("ask.savingToKB") : t("ask.saveToKB")}
                        </button>
                      )
                    )}
                  </div>
                )}
              </div>
            ))}
            {isAsking && (
              <div className="flex items-center gap-2 px-3 py-2">
                <Loader size={14} className="animate-spin text-orange-500" />
                <span className="text-xs" style={{ color: "var(--color-text-muted)" }}>{t("ask.thinking")}</span>
              </div>
            )}
            <div ref={messagesEndRef} />
          </div>

          {/* Input */}
          <div className="flex-shrink-0 p-3 border-t" style={{ borderColor: "var(--color-border, #E7E5E4)" }}>
            <div className="flex gap-2">
              <input
                type="text"
                value={input}
                onChange={(e) => setInput(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); handleSend(); } }}
                placeholder={t("ask.inputPlaceholder")}
                className="flex-1 px-3 py-2 rounded-lg text-sm outline-none"
                style={{
                  backgroundColor: "var(--color-surface-raised, #F5F5F0)",
                  border: "1px solid var(--color-border, #E7E5E4)",
                  color: "var(--color-text-primary)",
                }}
              />
              <button
                onClick={handleSend}
                disabled={isAsking || !input.trim()}
                className="px-3 py-2 rounded-lg text-white text-sm font-medium transition-all
                  disabled:opacity-40 disabled:cursor-not-allowed"
                style={{ backgroundColor: "#F97316" }}
              >
                {isAsking ? <Loader size={14} className="animate-spin" /> : <Send size={14} />}
              </button>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
