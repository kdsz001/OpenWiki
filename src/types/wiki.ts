export type WikiPageType =
  | "concept"
  | "entity"
  | "source"
  | "comparison"
  | "overview"
  | "case"
  | "theme"
  | "dashboard"
  | "qa";
export type WikiPageStatus = "active" | "needs_recompile" | "draft" | "archived";
export type WikiEdgeRelation = "related" | "part_of" | "contradicts" | "extends" | "compares";

export interface WikiPage {
  id: string;
  title: string;
  slug: string;
  page_type: WikiPageType;
  body_markdown: string;
  summary?: string;
  tags?: string;
  status: WikiPageStatus;
  confidence: number;
  created_at: string;
  updated_at: string;
  last_compiled_at?: string;
  source_message_id?: string | null;
}

export interface WikiPageSource {
  id: number;
  page_id: string;
  content_id: string;
  compile_hash: string;
  source_status: "active" | "stale" | "deleted";
  contributed_at: string;
}

export interface WikiEdge {
  id: number;
  source_page_id: string;
  target_page_id: string;
  relation: WikiEdgeRelation;
  weight: number;
  created_at: string;
}

export interface WikiConversation {
  id: string;
  question: string;
  answer: string;
  pages_used: string;
  saved_as_page?: string;
  model_used?: string;
  created_at: string;
}

export interface WikiLintResult {
  id: number;
  lint_type: string;
  severity: "info" | "warning" | "critical";
  title: string;
  description: string;
  page_ids: string;
  status: string;
  created_at: string;
}

export interface WikiStats {
  total_pages: number;
  total_edges: number;
  total_sources: number;
  needs_recompile: number;
  lint_open: number;
}

export interface WikiGraphNode {
  id: string;
  title: string;
  page_type: WikiPageType;
  status: WikiPageStatus;
  confidence: number;
  edge_count: number;
}

export interface WikiGraphData {
  nodes: WikiGraphNode[];
  edges: { source: string; target: string; relation: string; weight: number }[];
}

export interface WikiChatSession {
  id: string;
  title?: string;
  created_at: string;
  updated_at: string;
}

export interface WikiChatMessage {
  id: string;
  session_id: string;
  role: "user" | "assistant";
  content: string;
  pages_used?: string;
  source_mode?: "knowledge_base" | "mixed" | "ai_only";
  turn_index: number;
  created_at: string;
}
