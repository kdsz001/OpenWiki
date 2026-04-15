import { invoke } from "@tauri-apps/api/core";
import type {
  WikiPage,
  WikiPageSource,
  WikiConversation,
  WikiLintResult,
  WikiStats,
  WikiGraphData,
} from "../types/wiki";

// ===== Browse =====

export async function getWikiPages(opts?: {
  page_type?: string;
  limit?: number;
  offset?: number;
}): Promise<WikiPage[]> {
  return invoke("get_wiki_pages", {
    pageType: opts?.page_type ?? null,
    limit: opts?.limit ?? 100,
    offset: opts?.offset ?? 0,
  });
}

export async function getWikiPage(id: string): Promise<WikiPage | null> {
  return invoke("get_wiki_page", { id });
}

export async function resolveWikiLink(target: string): Promise<WikiPage | null> {
  return invoke("resolve_wiki_link", { target });
}

export async function searchWiki(query: string): Promise<WikiPage[]> {
  return invoke("search_wiki", { query });
}

export async function getPageThemes(pageId: string): Promise<WikiPage[]> {
  return invoke("get_page_themes", { pageId });
}

export async function getWikiStats(): Promise<WikiStats> {
  return invoke("get_wiki_stats");
}

export async function deleteWikiPage(id: string): Promise<void> {
  return invoke("delete_wiki_page", { id });
}

// ===== Graph =====

export async function getWikiGraph(): Promise<WikiGraphData> {
  return invoke("get_wiki_graph");
}

// ===== Compile =====

export async function compileContentToWiki(contentId: string): Promise<string[]> {
  return invoke("compile_content_to_wiki", { contentId });
}

export async function compileContentsToWiki(contentIds: string[]): Promise<{
  processed: number;
  compiled: number;
  failed: number;
  touched_pages: number;
  page_ids: string[];
}> {
  return invoke("compile_contents_to_wiki", { contentIds });
}

export async function triggerWikiAutoCompile(): Promise<{
  processed: number;
  compiled: number;
  errors: number;
  up_to_date: number;
  remaining: number;
}> {
  return invoke("trigger_wiki_auto_compile");
}

export async function syncLocalWiki(path: string): Promise<{
  root: string;
  pages_found: number;
  created: number;
  updated: number;
  removed: number;
  edges_created: number;
  counts: Record<string, number>;
}> {
  return invoke("sync_local_wiki", { path });
}

// ===== Q&A (multi-turn chat) =====

export async function wikiAsk(sessionId: string, question: string): Promise<{
  message_id: string;
  answer: string;
  pages_used: { id: string; title: string }[];
  source_mode: "knowledge_base" | "mixed" | "ai_only";
  confidence: number;
}> {
  return invoke("wiki_ask", { sessionId, question });
}

export async function getChatSessions(limit?: number): Promise<import("../types/wiki").WikiChatSession[]> {
  return invoke("get_chat_sessions", { limit: limit ?? 20 });
}

export async function getChatMessages(sessionId: string): Promise<import("../types/wiki").WikiChatMessage[]> {
  return invoke("get_chat_messages", { sessionId });
}

export async function deleteChatSession(sessionId: string): Promise<void> {
  return invoke("delete_chat_session", { sessionId });
}

export async function saveMessageAsPage(sessionId: string, messageId: string): Promise<WikiPage> {
  return invoke("save_message_as_page", { sessionId, messageId });
}

export async function getSavedMessageIds(messageIds: string[]): Promise<string[]> {
  return invoke("get_saved_message_ids", { messageIds });
}

// Legacy
export async function getWikiConversations(limit?: number): Promise<WikiConversation[]> {
  return invoke("get_wiki_conversations", { limit: limit ?? 20 });
}

// ===== Tag-based linking =====

export async function wikiLinkByTags(): Promise<{ edges_created: number }> {
  return invoke("wiki_link_by_tags");
}

// ===== Lint =====

export async function triggerWikiLint(): Promise<WikiLintResult[]> {
  return invoke("trigger_wiki_lint");
}

export async function getWikiLintResults(): Promise<WikiLintResult[]> {
  return invoke("get_wiki_lint_results");
}

export async function wikiLintKeep(lintId: number): Promise<void> {
  return invoke("wiki_lint_keep", { lintId });
}

export async function wikiLintDelete(lintId: number): Promise<void> {
  return invoke("wiki_lint_delete", { lintId });
}

export async function wikiLintRecompile(lintId: number): Promise<void> {
  return invoke("wiki_lint_recompile", { lintId });
}

// ===== Sources =====

export async function getPageSources(pageId: string): Promise<WikiPageSource[]> {
  return invoke("get_page_sources", { pageId });
}

export async function getContentWikiPages(contentId: string): Promise<WikiPage[]> {
  return invoke("get_content_wiki_pages", { contentId });
}
