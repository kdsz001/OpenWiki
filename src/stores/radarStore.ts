import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import {
  getAttentionInsights,
  triggerAttentionAnalysis,
  type RadarStatus,
  type BriefingAnalysis,
  type RadarReport,
} from "../services/radarService";

interface RadarProgress {
  stage: string;
  current: number;
  total: number;
  selected_items?: number;
  total_items?: number;
}

interface RadarState {
  status: RadarStatus["status"];
  analysis: BriefingAnalysis | null;
  report: RadarReport | null;
  contentCount: number;
  hasNewContent: boolean;
  errorMessage: string | null;
  isLoading: boolean;
  progress: RadarProgress | null;

  loadRadar: () => Promise<void>;
  triggerAnalysis: () => Promise<void>;
  setupEventListener: () => Promise<() => void>;
}

export const useRadarStore = create<RadarState>((set, get) => ({
  status: "empty",
  analysis: null,
  report: null,
  contentCount: 0,
  hasNewContent: false,
  errorMessage: null,
  isLoading: true,
  progress: null,

  loadRadar: async () => {
    set({ isLoading: true });
    try {
      const result = await getAttentionInsights();
      let analysis: BriefingAnalysis | null = null;
      let report: RadarReport | null = null;
      let errorMessage = result.insight?.error_message ?? null;
      let status = result.status;

      if (result.insight?.analysis_json) {
        try {
          const raw = JSON.parse(result.insight.analysis_json);
          const normalized = normalizeAnalysis(raw);
          if (normalized.type === "v3") {
            report = normalized.report;
          } else if (normalized.type === "v2") {
            analysis = normalized.analysis;
          } else {
            status = "error";
            errorMessage = "分析结果格式无法识别，请重新生成";
          }
        } catch {
          status = "error";
          errorMessage = "分析结果解析失败，请重新生成";
        }
      }

      set({
        status,
        analysis,
        report,
        contentCount: result.insight?.content_count ?? 0,
        hasNewContent: result.has_new_content,
        errorMessage,
        isLoading: false,
        progress: status === "analyzing" ? get().progress : null,
      });

      // Auto-trigger analysis if stale or empty
      if (status === "stale" || status === "empty") {
        get().triggerAnalysis();
      }
    } catch (e) {
      set({
        isLoading: false,
        status: "error",
        errorMessage: e instanceof Error ? e.message : String(e),
        progress: null,
      });
    }
  },

  triggerAnalysis: async () => {
    set({ status: "analyzing", progress: null });
    try {
      await triggerAttentionAnalysis();
    } catch (e) {
      set({
        status: "error",
        errorMessage: e instanceof Error ? e.message : String(e),
        progress: null,
      });
    }
  },

  setupEventListener: async () => {
    try {
      const unlistenProgress = await listen<RadarProgress | string>("attention-analysis-progress", (event) => {
        const payload = event.payload;
        if (typeof payload === "string") {
          set({
            status: "analyzing",
            progress: { stage: payload, current: 0, total: 0 },
          });
          return;
        }
        set({ status: "analyzing", progress: payload });
      });
      const unlisten = await listen<string>("attention-analysis-complete", () => {
        set({ progress: null });
        get().loadRadar();
      });
      return () => {
        unlistenProgress();
        unlisten();
      };
    } catch (e) {
      console.error("Failed to setup radar event listener:", e);
      return () => {};
    }
  },
}));

/* eslint-disable @typescript-eslint/no-explicit-any */
type NormalizedResult =
  | { type: "v3"; report: RadarReport }
  | { type: "v2"; analysis: BriefingAnalysis }
  | { type: "none"; analysis: null; report: null };

function normalizeAnalysis(raw: any): NormalizedResult {
  // v3 format: has "at_a_glance" key → RadarReport
  if (raw.at_a_glance && Array.isArray(raw.at_a_glance)) {
    return { type: "v3", report: raw as RadarReport };
  }

  // v2 format: has "topics" key
  if (raw.topics && Array.isArray(raw.topics)) {
    return {
      type: "v2",
      analysis: {
        format_version: raw.format_version ?? 2,
        topics: raw.topics.map((t: any) => ({
          id: t.id ?? "",
          rank: t.rank ?? 1,
          insight_title: t.insight_title ?? "",
          deep_analysis: t.deep_analysis ?? "",
          key_findings: Array.isArray(t.key_findings) ? t.key_findings : [],
          suggestion: t.suggestion ?? null,
          evidence_indices: Array.isArray(t.evidence_indices) ? t.evidence_indices : [],
          content_count: t.content_count ?? 0,
          span_days: t.span_days ?? 0,
          trend: t.trend ?? "stable",
          tag: t.tag ?? "核心关注",
        })),
        meta: {
          total_content: raw.meta?.total_content ?? 0,
          window_days: raw.meta?.window_days ?? 14,
          analysis_depth: raw.meta?.analysis_depth ?? "deep",
        },
        id_map: raw.id_map ?? {},
      },
    };
  }

  // v1 format fallback: has "analysis.recurring_threads"
  const a = raw.analysis || raw;
  if (a.recurring_threads) {
    const topics: any[] = [];
    (a.recurring_threads || []).forEach((t: any, i: number) => {
      topics.push({
        id: `v1_thread_${i}`,
        rank: i + 1,
        insight_title: t.title || t.topic || "",
        deep_analysis: t.why_now || t.summary || t.description || "",
        key_findings: [],
        suggestion: null,
        evidence_indices: (t.evidence || []).map((e: any) => e.index ?? 0),
        content_count: (t.evidence || []).length,
        span_days: 14,
        trend: "stable" as const,
        tag: "核心关注" as const,
      });
    });
    return {
      type: "v2",
      analysis: {
        format_version: 1,
        topics: topics.slice(0, 3),
        meta: { total_content: 0, window_days: 14, analysis_depth: "shallow" },
        id_map: raw.id_map ?? {},
      },
    };
  }

  return { type: "none", analysis: null, report: null };
}
/* eslint-enable @typescript-eslint/no-explicit-any */
