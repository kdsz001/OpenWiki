import { useState, useEffect, Component, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw, Key, Target, Search } from "lucide-react";
import { useRadarStore } from "../../stores/radarStore";
import type {
  Glance,
  InfoDiet,
  SubconsciousItem,
  Graveyard,
  BlindSpot,
  Action,
  HeatmapDay,
  TopicItem,
  Verdict,
  Footer,
  BriefingTopic,
} from "../../services/radarService";

const ACCENT = "#F97316";

// Error boundary
class RadarErrorBoundary extends Component<{ children: ReactNode; errorTitle: string; retryLabel: string }, { error: string | null }> {
  state = { error: null as string | null };
  static getDerivedStateFromError(error: Error) {
    return { error: error.message };
  }
  render() {
    if (this.state.error) {
      return (
        <div className="px-5 py-8" style={{ color: "var(--color-text-primary)" }}>
          <h2 className="text-lg font-bold mb-2">{this.props.errorTitle}</h2>
          <p style={{ fontSize: 13, color: "var(--color-text-secondary)" }}>{this.state.error}</p>
          <button
            onClick={() => this.setState({ error: null })}
            className="mt-3 font-medium"
            style={{ fontSize: 13, color: ACCENT }}
          >
            {this.props.retryLabel}
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

export function RadarView() {
  const { t } = useTranslation("digest");
  return (
    <RadarErrorBoundary errorTitle={t("radar.errorTitle")} retryLabel={t("radar.retry")}>
      <RadarViewInner />
    </RadarErrorBoundary>
  );
}

function RadarViewInner() {
  const { t } = useTranslation("digest");
  const {
    status,
    analysis,
    report,
    errorMessage,
    isLoading,
    progress,
    loadRadar,
    triggerAnalysis,
    setupEventListener,
  } = useRadarStore();

  useEffect(() => {
    loadRadar();
    let unlisten: (() => void) | undefined;
    setupEventListener().then((fn) => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, [loadRadar, setupEventListener]);

  const isAnalyzing = status === "analyzing";
  const hasReport = report !== null;
  const hasLegacy = analysis !== null && (analysis.topics?.length ?? 0) > 0;
  const hasFindings = hasReport || hasLegacy;

  return (
    <div className="overflow-y-auto" style={{ height: "calc(100vh - 44px)", color: "var(--color-text-primary)" }}>

      {/* Header */}
      <div className="px-5 pt-5 pb-3">
        <div className="flex items-center justify-between mb-1">
          <h2
            style={{
              fontSize: 22,
              fontFamily: "'Cabinet Grotesk', sans-serif",
              fontWeight: 700,
              color: "var(--color-text-primary)",
              letterSpacing: "-0.3px",
            }}
          >
            {t("radar.title")}
          </h2>
          <div className="flex items-center gap-1">
            <button
              onClick={() => triggerAnalysis()}
              disabled={isAnalyzing}
              className="p-2 rounded-lg text-stone-400 dark:text-stone-500 hover:text-stone-600 dark:hover:text-stone-300
                         hover:bg-stone-100 dark:hover:bg-white/[0.08]
                         disabled:opacity-40 disabled:cursor-not-allowed transition-all"
              title={t("radar.refreshTitle")}
            >
              <RefreshCw size={18} strokeWidth={2} className={isAnalyzing ? "animate-spin" : ""} />
            </button>
          </div>
        </div>
        {!isLoading && hasFindings && (
          <p style={{ fontSize: 13, color: "var(--color-text-muted)" }}>
            {t("radar.subtitle")}
          </p>
        )}
      </div>

      <div className="px-5 pb-8">
        {/* Loading */}
        {isLoading && <LoadingSkeleton />}

        {/* Empty states */}
        {!isLoading && status === "no_api_key" && (
          <EmptyState
            icon={<Key size={48} className="text-stone-300 dark:text-stone-600 mb-4" strokeWidth={1.5} />}
            title={t("radar.emptyNeedApiKey.title")}
            desc={t("radar.emptyNeedApiKey.desc")}
          />
        )}
        {!isLoading && status === "not_enough_content" && (
          <EmptyState
            icon={<Target size={48} className="text-stone-300 dark:text-stone-600 mb-4" strokeWidth={1.5} />}
            title={t("radar.emptyNotEnough.title")}
            desc={t("radar.emptyNotEnough.desc")}
          />
        )}
        {!isLoading && !isAnalyzing && !hasFindings &&
         status !== "no_api_key" && status !== "not_enough_content" &&
         status !== "error" && (
          <EmptyState
            icon={<Search size={48} className="text-stone-300 dark:text-stone-600 mb-4" strokeWidth={1.5} />}
            title={t("radar.emptyScattered.title")}
            desc={t("radar.emptyScattered.desc")}
          />
        )}
        {!isLoading && status === "error" && (
          <div className="rounded-xl p-4 mt-4" style={{ backgroundColor: "var(--color-surface)", border: "1px solid var(--color-border)" }}>
            <p className="text-red-700 dark:text-red-400 mb-2" style={{ fontSize: 13 }}>
              {errorMessage || t("radar.errorDefault")}
            </p>
            <button onClick={() => triggerAnalysis()} className="font-medium hover:underline" style={{ fontSize: 13, color: ACCENT }}>
              {t("radar.reanalyze")}
            </button>
          </div>
        )}

        {!isLoading && isAnalyzing && (
          <div
            className="rounded-xl px-4 py-3 mb-4"
            style={{ backgroundColor: "var(--color-surface)", border: "1px solid var(--color-border)" }}
          >
            <p style={{ fontSize: 13, color: "var(--color-text-secondary)" }}>
              {formatRadarProgress(t, progress)}
            </p>
          </div>
        )}

        {/* V3 RadarReport */}
        {!isLoading && hasReport && report && (
          <div>
            <StatsGrid report={report} />
            <Section num="01" title={t("radar.sections.atAGlance")} subtitle={t("radar.sections.atAGlanceSubtitle")}>
              <AtAGlanceBody items={report.at_a_glance} />
            </Section>
            <Section num="02" title={t("radar.sections.infoDiet")} subtitle={t("radar.sections.infoDietSubtitle")}>
              <InfoDietBody diet={report.info_diet} />
            </Section>
            <Section num="03" title={t("radar.sections.subconscious")} subtitle={t("radar.sections.subconsciousSubtitle")}>
              <SubconsciousBody items={report.subconscious} />
            </Section>
            <Section num="04" title={t("radar.sections.graveyard")} subtitle={t("radar.sections.graveyardSubtitle")}>
              <GraveyardBody graveyard={report.graveyard} />
            </Section>
            <Section num="05" title={t("radar.sections.blindSpots")} subtitle={t("radar.sections.blindSpotsSubtitle")}>
              <BlindSpotsBody items={report.blind_spots} />
            </Section>
            <Section num="06" title={t("radar.sections.actions")} subtitle={t("radar.sections.actionsSubtitle")}>
              <ActionsBody items={report.actions} />
            </Section>
            <Section num="⊹" title={t("radar.sections.heatmap")} subtitle={t("radar.sections.heatmapSubtitle")}>
              <HeatmapBody days={report.heatmap} />
              <div style={{ height: 1, backgroundColor: "var(--color-border)", margin: "16px 0" }} />
              <div style={{ fontSize: 11, color: "var(--color-text-muted)", textTransform: "uppercase", marginBottom: 10 }}>{t("radar.sections.topicDistribution")}</div>
              <TopicCloudBody items={report.topic_cloud} />
            </Section>
            <Section num="07" title={t("radar.sections.verdict")} subtitle={t("radar.sections.verdictSubtitle")}>
              <VerdictBody verdict={report.verdict} />
            </Section>
            <ReportFooter footer={report.footer} />

            {isAnalyzing && (
              <div className="text-center py-6">
                <RefreshCw size={16} className="animate-spin text-stone-400 mx-auto mb-2" />
                <p className="text-stone-400" style={{ fontSize: 13 }}>{t("radar.updating")}</p>
              </div>
            )}
          </div>
        )}

        {/* V2 Legacy fallback */}
        {!isLoading && !hasReport && hasLegacy && analysis && (
          <>
            <LegacyBriefingHero topic={analysis.topics[0]} />
            {analysis.topics.length > 1 && (
              <div className="grid grid-cols-2 gap-3 mb-6">
                {analysis.topics.slice(1).map((topic) => (
                  <LegacyBriefingSecondary key={topic.id} topic={topic} />
                ))}
              </div>
            )}
          </>
        )}

        {/* Analyzing with no data */}
        {!isLoading && isAnalyzing && !hasFindings && <AnalyzingSkeleton />}

        {/* Wiki health section */}
        <WikiLintSectionLazy />
      </div>
    </div>
  );
}

function formatRadarProgress(
  t: (key: string, options?: Record<string, unknown>) => string,
  progress: {
    stage: string;
    current: number;
    total: number;
    selected_items?: number;
    total_items?: number;
  } | null,
) {
  if (!progress) {
    return t("radar.analyzing");
  }

  const selected = progress.selected_items ?? 0;
  const totalItems = progress.total_items ?? selected;

  switch (progress.stage) {
    case "planning":
      return t("radar.progress.planning", { selected, total: totalItems, batches: progress.total });
    case "thinking":
      return t("radar.progress.thinking", { selected, total: totalItems });
    case "batching":
      return t("radar.progress.batching", { current: progress.current, total: progress.total, selected });
    case "aggregating":
      return t("radar.progress.aggregating", { total: progress.total });
    case "generating":
      return t("radar.progress.generating");
    default:
      return t("radar.analyzing");
  }
}

function WikiLintSectionLazy() {
  const [WikiLint, setWikiLint] = useState<React.ComponentType<{ compact?: boolean }> | null>(null);
  useEffect(() => {
    import("../wiki/WikiLintSection").then((m) => setWikiLint(() => m.WikiLintSection));
  }, []);
  if (!WikiLint) return null;
  return (
    <div className="mt-6 pt-4" style={{ borderTop: "1px solid var(--color-border, #E7E5E4)" }}>
      <WikiLint compact />
    </div>
  );
}

// ====================================================================
// Stats Grid (top 5 numbers)
// ====================================================================

function StatsGrid({ report }: { report: { meta: { total_items: number; active_days: number; annotated_items: number; annotation_rate: string; source_count: number }; footer: { total_days: number } } }) {
  const { t } = useTranslation("digest");
  const { meta } = report;
  const stats = [
    { n: meta.total_items, l: t("radar.stats.savedItems") },
    { n: meta.active_days, l: t("radar.stats.activeDays") },
    { n: meta.annotated_items, l: t("radar.stats.annotated") },
    { n: meta.annotation_rate, l: t("radar.stats.annotationRate") },
    { n: meta.source_count, l: t("radar.stats.sources") },
  ];

  return (
    <div
      className="grid grid-cols-5 mb-6 overflow-hidden"
      style={{ borderRadius: 14, border: "1px solid var(--color-border)" }}
    >
      {stats.map((s, i) => (
        <div
          key={i}
          className="text-center py-4 px-2"
          style={{
            backgroundColor: "var(--color-surface)",
            borderRight: i < 4 ? "1px solid var(--color-border)" : undefined,
          }}
        >
          <div
            style={{
              fontSize: 26,
              fontWeight: 800,
              fontFamily: "'JetBrains Mono', monospace",
              color: ACCENT,
            }}
          >
            {s.n}
          </div>
          <div style={{ fontSize: 10, color: "var(--color-text-muted)", marginTop: 4, textTransform: "uppercase" }}>
            {s.l}
          </div>
        </div>
      ))}
    </div>
  );
}

// ====================================================================
// Section wrapper with numbered tag
// ====================================================================

function Section({ num, title, subtitle, children }: { num: string; title: string; subtitle: string; children: ReactNode }) {
  return (
    <div className="mb-5" style={{ backgroundColor: "var(--color-surface)", border: "1px solid var(--color-border)", borderRadius: 16 }}>
      {/* Section header */}
      <div className="flex items-center gap-3 px-5 py-3" style={{ borderBottom: "1px solid var(--color-border)" }}>
        <span
          style={{
            fontSize: 10,
            fontWeight: 700,
            color: ACCENT,
            backgroundColor: `${ACCENT}15`,
            border: `1px solid ${ACCENT}30`,
            borderRadius: 6,
            padding: "2px 8px",
            flexShrink: 0,
          }}
        >
          {num}
        </span>
        <span style={{ fontSize: 16, fontWeight: 700, color: "var(--color-text-primary)" }}>
          {title}{" "}
          <span style={{ color: ACCENT }}>{subtitle}</span>
        </span>
      </div>
      {/* Section body */}
      <div className="px-5 py-4">{children}</div>
    </div>
  );
}

// ====================================================================
// 01 At a Glance
// ====================================================================

function AtAGlanceBody({ items }: { items: Glance[] }) {
  return (
    <div className="space-y-3">
      {items.map((item, i) => (
        <div
          key={i}
          className="rounded-xl p-4"
          style={{
            backgroundColor: `${ACCENT}08`,
            border: `1px solid ${ACCENT}20`,
          }}
        >
          <p style={{ fontSize: 14, lineHeight: 1.8, color: "var(--color-text-secondary)" }}>
            <HighlightText text={item.text} highlight={item.highlight} />
          </p>
        </div>
      ))}
    </div>
  );
}

// ====================================================================
// 02 Info Diet
// ====================================================================

function InfoDietBody({ diet }: { diet: InfoDiet }) {
  const { t } = useTranslation("digest");
  const maxCount = Math.max(...diet.sources.map((s) => s.count), 1);

  return (
    <>
      <div style={{ fontSize: 11, color: "var(--color-text-muted)", textTransform: "uppercase", marginBottom: 10 }}>{t("radar.infoDiet.sourceDistribution")}</div>
      <div className="space-y-2 mb-4">
        {diet.sources.map((src) => (
          <div key={src.name} className="flex items-center gap-3">
            <span className="w-20 text-right shrink-0" style={{ fontSize: 12, color: "var(--color-text-secondary)" }}>
              {src.name}
            </span>
            <div className="flex-1 rounded-md overflow-hidden" style={{ height: 24, backgroundColor: "var(--color-surface-raised, #F5F5F0)" }}>
              <div
                className="h-full rounded-md flex items-center justify-end px-2"
                style={{
                  width: `${Math.max((src.count / maxCount) * 100, 8)}%`,
                  background: sourceGradient(src.color),
                }}
              >
                <span style={{ fontSize: 11, fontWeight: 700, color: "rgba(255,255,255,0.9)" }}>{src.count}{t("radar.infoDiet.itemsUnit")}</span>
              </div>
            </div>
          </div>
        ))}
      </div>

      {/* Metrics */}
      <div className="grid grid-cols-2 gap-3 mb-4">
        <MiniCard title={t("radar.infoDiet.depthVsFragment")} value={diet.depth_ratio.label} percent={parsePercent(diet.depth_ratio.label)} />
        <MiniCard
          title={t("radar.infoDiet.dietBias")}
          value={`${diet.dominant_topic.name} ${diet.dominant_topic.percent.toFixed(0)}%`}
          percent={diet.dominant_topic.percent}
        />
      </div>

      {/* Alert */}
      {diet.alert && (
        <div
          className="flex gap-2 rounded-xl px-4 py-3"
          style={{
            fontSize: 13,
            backgroundColor: "rgba(245, 158, 11, 0.08)",
            border: "1px solid rgba(245, 158, 11, 0.2)",
            color: "var(--color-text-secondary)",
          }}
        >
          <span>⚠️</span>
          <span>{diet.alert}</span>
        </div>
      )}
    </>
  );
}

function MiniCard({ title, value, percent }: { title: string; value: string; percent: number }) {
  return (
    <div className="rounded-xl p-3" style={{ backgroundColor: "var(--color-surface-raised, #F5F5F0)", border: "1px solid var(--color-border)" }}>
      <div style={{ fontSize: 10, color: "var(--color-text-muted)", textTransform: "uppercase", marginBottom: 6 }}>{title}</div>
      <div style={{ fontSize: 13, fontWeight: 600, color: "var(--color-text-primary)" }}>{value}</div>
      <div className="mt-2 rounded-full overflow-hidden" style={{ height: 4, backgroundColor: "var(--color-border)" }}>
        <div className="h-full rounded-full" style={{ width: `${Math.min(percent, 100)}%`, background: `linear-gradient(90deg, ${ACCENT}, #FB923C)` }} />
      </div>
    </div>
  );
}

// ====================================================================
// 03 Subconscious
// ====================================================================

function SubconsciousBody({ items }: { items: SubconsciousItem[] }) {
  const { t } = useTranslation("digest");
  return (
    <div className="space-y-3">
      {items.map((item, i) => (
        <div
          key={i}
          className="rounded-r-xl py-3 px-4"
          style={{
            backgroundColor: "var(--color-surface-raised, #F5F5F0)",
            borderLeft: `3px solid ${ACCENT}`,
          }}
        >
          <div className="flex items-start justify-between gap-2 mb-1">
            <span style={{ fontSize: 14, fontWeight: 700, color: "var(--color-text-primary)" }}>
              🎯 {item.title}
            </span>
            {item.evidence_count != null && (
              <span style={{ fontSize: 11, fontFamily: "'JetBrains Mono', monospace", color: ACCENT, whiteSpace: "nowrap" }}>
                {t("radar.subconscious.evidenceCount", { count: item.evidence_count })}
              </span>
            )}
          </div>
          <p style={{ fontSize: 13, lineHeight: 1.6, color: "var(--color-text-secondary)" }}>{item.body}</p>
        </div>
      ))}
    </div>
  );
}

// ====================================================================
// 04 Graveyard
// ====================================================================

function GraveyardBody({ graveyard }: { graveyard: Graveyard }) {
  const { t } = useTranslation("digest");
  return (
    <>
      {/* Alert */}
      <div
        className="flex gap-2 rounded-xl px-4 py-3 mb-4"
        style={{
          fontSize: 13,
          backgroundColor: "rgba(245, 158, 11, 0.08)",
          border: "1px solid rgba(245, 158, 11, 0.2)",
          color: "var(--color-text-secondary)",
        }}
      >
        <span>🪦</span>
        <span>{graveyard.alert}</span>
      </div>

      <div style={{ fontSize: 11, color: "var(--color-text-muted)", textTransform: "uppercase", marginBottom: 10 }}>{t("radar.graveyard.worthReading")}</div>

      <div className="space-y-3">
        {graveyard.top_picks.map((pick) => (
          <div
            key={pick.rank}
            className="rounded-xl p-4 flex gap-3"
            style={{ backgroundColor: "var(--color-surface-raised, #F5F5F0)", border: "1px solid var(--color-border)" }}
          >
            {/* Numbered circle */}
            <div
              className="shrink-0 flex items-center justify-center"
              style={{
                width: 28,
                height: 28,
                borderRadius: "50%",
                background: `linear-gradient(135deg, ${ACCENT}, #EA580C)`,
                color: "#fff",
                fontSize: 13,
                fontWeight: 800,
              }}
            >
              {pick.rank}
            </div>
            <div className="min-w-0 flex-1">
              <div style={{ fontSize: 14, fontWeight: 700, color: "var(--color-text-primary)", marginBottom: 4 }}>{pick.title}</div>
              <p style={{ fontSize: 12, lineHeight: 1.6, color: "var(--color-text-secondary)", marginBottom: 8 }}>{pick.reason}</p>
              {pick.tags.length > 0 && (
                <div className="flex flex-wrap gap-1.5">
                  {pick.tags.map((tag) => (
                    <span
                      key={tag}
                      className="rounded-full px-2.5 py-0.5"
                      style={{
                        fontSize: 10,
                        color: ACCENT,
                        backgroundColor: `${ACCENT}10`,
                        border: `1px solid ${ACCENT}25`,
                      }}
                    >
                      {tag}
                    </span>
                  ))}
                </div>
              )}
            </div>
          </div>
        ))}
      </div>
    </>
  );
}

// ====================================================================
// 05 Blind Spots
// ====================================================================

function BlindSpotsBody({ items }: { items: BlindSpot[] }) {
  return (
    <div className="space-y-3">
      {items.map((item, i) => (
        <div
          key={i}
          className="rounded-xl p-4"
          style={{
            backgroundColor: `${ACCENT}08`,
            border: `1px solid ${ACCENT}20`,
          }}
        >
          <h4 className="mb-1" style={{ fontSize: 14, fontWeight: 700, color: "var(--color-text-primary)" }}>{item.title}</h4>
          <p style={{ fontSize: 13, lineHeight: 1.6, color: "var(--color-text-secondary)" }}>{item.body}</p>
        </div>
      ))}
    </div>
  );
}

// ====================================================================
// 06 Actions
// ====================================================================

function ActionsBody({ items }: { items: Action[] }) {
  return (
    <div className="space-y-3">
      {items.map((item, i) => (
        <div
          key={i}
          className="rounded-xl p-4 flex gap-3"
          style={{ backgroundColor: "var(--color-surface-raised, #F5F5F0)", border: "1px solid var(--color-border)" }}
        >
          <span style={{ fontSize: 20, lineHeight: 1, flexShrink: 0 }}>{item.icon}</span>
          <div className="flex-1 min-w-0">
            <div style={{ fontSize: 14, fontWeight: 700, color: "var(--color-text-primary)", marginBottom: 4 }}>{item.title}</div>
            <p style={{ fontSize: 13, lineHeight: 1.5, color: "var(--color-text-secondary)", marginBottom: 8 }}>{item.desc}</p>
            <div className="flex items-center gap-2">
              <span
                className="rounded-full px-2.5 py-0.5"
                style={{ fontSize: 10, color: ACCENT, backgroundColor: `${ACCENT}10`, border: `1px solid ${ACCENT}25` }}
              >
                {item.ref}
              </span>
              <span
                className="rounded-full px-2.5 py-0.5"
                style={{ fontSize: 10, color: "#10B981", backgroundColor: "rgba(16,185,129,0.08)", border: "1px solid rgba(16,185,129,0.18)" }}
              >
                ⏱ {item.time}
              </span>
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

// ====================================================================
// Heatmap
// ====================================================================

function HeatmapBody({ days }: { days: HeatmapDay[] }) {
  const maxCount = Math.max(...days.map((d) => d.count), 1);

  return (
    <div className="flex gap-2 flex-wrap">
      {days.map((day) => {
        const intensity = day.count / maxCount;
        const isPeak = intensity > 0.8 && day.count > 0;
        const bg = day.count === 0
          ? "var(--color-surface-raised, #F5F5F0)"
          : `rgba(249, 115, 22, ${0.15 + intensity * 0.85})`;

        return (
          <div key={day.date} className="flex flex-col items-center gap-1">
            <div
              className="flex items-center justify-center rounded-lg"
              style={{
                width: 40,
                height: 40,
                backgroundColor: bg,
                border: day.count === 0 ? "1px solid var(--color-border)" : undefined,
              }}
            >
              <span
                style={{
                  fontSize: 13,
                  fontWeight: 700,
                  fontFamily: "'JetBrains Mono', monospace",
                  color: intensity > 0.4 ? "#fff" : "var(--color-text-muted)",
                }}
              >
                {day.count > 0 ? day.count : ""}
              </span>
            </div>
            <span
              style={{
                fontSize: 9,
                fontFamily: "'JetBrains Mono', monospace",
                color: "var(--color-text-muted)",
                whiteSpace: "nowrap",
              }}
            >
              {formatHeatDate(day.date)}{isPeak ? "⚡" : ""}
            </span>
          </div>
        );
      })}
    </div>
  );
}

// ====================================================================
// Topic Cloud (pill style)
// ====================================================================

function TopicCloudBody({ items }: { items: TopicItem[] }) {
  return (
    <div className="flex flex-wrap gap-2">
      {items.map((item) => (
        <span
          key={item.name}
          className="rounded-full px-3 py-1"
          style={{
            fontSize: 13,
            color: ACCENT,
            backgroundColor: `${ACCENT}10`,
            border: `1px solid ${ACCENT}25`,
          }}
        >
          {item.name} ({item.percent.toFixed(0)}%)
        </span>
      ))}
    </div>
  );
}

// ====================================================================
// 07 Verdict
// ====================================================================

function VerdictBody({ verdict }: { verdict: Verdict }) {
  return (
    <div
      className="rounded-xl py-6 px-5 text-center"
      style={{
        background: `linear-gradient(135deg, ${ACCENT}12, rgba(234, 88, 12, 0.08))`,
        border: `1px solid ${ACCENT}30`,
      }}
    >
      <p
        style={{
          fontSize: 18,
          fontWeight: 700,
          lineHeight: 1.7,
          fontFamily: "'Cabinet Grotesk', sans-serif",
          color: "var(--color-text-primary)",
        }}
      >
        <HighlightVerdict text={verdict.text} highlights={verdict.highlights} />
      </p>
    </div>
  );
}

// ====================================================================
// Footer
// ====================================================================

function ReportFooter({ footer }: { footer: Footer }) {
  const { t } = useTranslation("digest");
  return (
    <div className="text-center py-4 mt-2" style={{ borderTop: "1px solid var(--color-border)" }}>
      <div style={{ fontSize: 11, fontFamily: "'JetBrains Mono', monospace", color: "var(--color-text-muted)" }}>
        <strong>{t("radar.footer.brand")}</strong> · {footer.date_range} · {t("radar.footer.itemsCount", { count: footer.total })} · {t("radar.footer.activeDays", { active: footer.active_days, total: footer.total_days })}
      </div>
    </div>
  );
}

// ====================================================================
// Legacy v2 Briefing cards (fallback)
// ====================================================================

function LegacyBriefingHero({ topic }: { topic: BriefingTopic }) {
  const { t } = useTranslation("digest");
  return (
    <div className="rounded-xl p-4 mb-3" style={{ backgroundColor: "var(--color-surface)", border: "1px solid var(--color-border)" }}>
      <div className="flex items-center gap-1.5 mb-3">
        <span className="w-1.5 h-1.5 rounded-full bg-orange-400" />
        <span style={{ fontSize: 10, fontWeight: 700, textTransform: "uppercase", letterSpacing: "0.8px", color: ACCENT }}>{topic.tag}</span>
      </div>
      <h3 className="mb-3" style={{ fontSize: 18, fontWeight: 700, lineHeight: 1.4, fontFamily: "'Cabinet Grotesk', sans-serif" }}>{topic.insight_title}</h3>
      {topic.key_findings.length > 0 && (
        <div className="mb-3 space-y-2">
          {topic.key_findings.map((finding, i) => (
            <div key={i} className="flex gap-2" style={{ fontSize: 13, lineHeight: 1.5, color: "var(--color-text-secondary)" }}>
              <span style={{ fontFamily: "'JetBrains Mono', monospace", fontSize: 11, fontWeight: 600, color: ACCENT, minWidth: 18, paddingTop: 2 }}>{i + 1}</span>
              <span>{finding}</span>
            </div>
          ))}
        </div>
      )}
      {topic.suggestion && (
        <div className="rounded-lg p-3 mb-3" style={{ backgroundColor: `${ACCENT}10`, border: `1px solid ${ACCENT}30` }}>
          <div style={{ fontSize: 10, fontWeight: 700, textTransform: "uppercase", letterSpacing: "0.8px", color: ACCENT, marginBottom: 4 }}>{t("radar.legacy.suggestion")}</div>
          <div style={{ fontSize: 13, lineHeight: 1.5, color: "var(--color-text-secondary)" }}>{topic.suggestion}</div>
        </div>
      )}
      <div className="pt-3" style={{ borderTop: "1px solid var(--color-border)", fontSize: 11, color: "var(--color-text-muted)", fontFamily: "'JetBrains Mono', monospace" }}>
        {t("radar.legacy.contentCount", { count: topic.content_count })} · {t("radar.legacy.spanDays", { count: topic.span_days })}
      </div>
    </div>
  );
}

function LegacyBriefingSecondary({ topic }: { topic: BriefingTopic }) {
  const { t } = useTranslation("digest");
  const tagColor = topic.tag === t("insight.tag.emergingInterest") ? "#4ADE80" : "#3B82F6";
  const truncatedAnalysis = topic.deep_analysis.length > 80 ? topic.deep_analysis.slice(0, 80) + "..." : topic.deep_analysis;

  return (
    <div className="rounded-xl p-3" style={{ backgroundColor: "var(--color-surface)", border: "1px solid var(--color-border)" }}>
      <div className="flex items-center gap-1.5 mb-2">
        <span className="w-1 h-1 rounded-full" style={{ backgroundColor: tagColor }} />
        <span style={{ fontSize: 10, fontWeight: 700, textTransform: "uppercase", letterSpacing: "0.8px", color: tagColor }}>{topic.tag}</span>
      </div>
      <h4 className="mb-1.5" style={{ fontSize: 14, fontWeight: 600, lineHeight: 1.35 }}>{topic.insight_title}</h4>
      <p className="mb-2.5" style={{ fontSize: 12, lineHeight: 1.5, color: "var(--color-text-muted)" }}>{truncatedAnalysis}</p>
      <div style={{ fontSize: 10, color: "var(--color-text-muted)", fontFamily: "'JetBrains Mono', monospace" }}>
        {t("radar.legacy.itemsShort", { count: topic.content_count })} · {t("radar.legacy.daysShort", { count: topic.span_days })}
      </div>
    </div>
  );
}

// ====================================================================
// Helpers
// ====================================================================

function HighlightText({ text, highlight }: { text: string; highlight: string }) {
  if (!highlight) return <>{text}</>;
  const idx = text.indexOf(highlight);
  if (idx === -1) return <>{text}</>;
  return (
    <>
      {text.slice(0, idx)}
      <span style={{ color: ACCENT, fontWeight: 600 }}>{highlight}</span>
      {text.slice(idx + highlight.length)}
    </>
  );
}

function HighlightVerdict({ text, highlights }: { text: string; highlights: string[] }) {
  if (!highlights.length) return <>{text}</>;
  let parts: (string | { hl: string })[] = [text];
  for (const hl of highlights) {
    const newParts: (string | { hl: string })[] = [];
    for (const part of parts) {
      if (typeof part !== "string") { newParts.push(part); continue; }
      const idx = part.indexOf(hl);
      if (idx === -1) { newParts.push(part); } else {
        if (idx > 0) newParts.push(part.slice(0, idx));
        newParts.push({ hl });
        if (idx + hl.length < part.length) newParts.push(part.slice(idx + hl.length));
      }
    }
    parts = newParts;
  }
  return (
    <>
      {parts.map((p, i) =>
        typeof p === "string"
          ? <span key={i}>{p}</span>
          : <span key={i} style={{ color: ACCENT }}>{p.hl}</span>
      )}
    </>
  );
}

function sourceGradient(color: string): string {
  switch (color) {
    case "wechat": return "linear-gradient(90deg, #15803D, #22C55E)";
    case "chrome": return "linear-gradient(90deg, #1D4ED8, #3B82F6)";
    case "xiaoyun": return `linear-gradient(90deg, #EA580C, ${ACCENT})`;
    default: return "linear-gradient(90deg, #78716C, #A8A29E)";
  }
}

function formatHeatDate(date: string): string {
  // "2026-03-21" -> "3/21"
  const parts = date.split("-");
  if (parts.length === 3) return `${parseInt(parts[1])}/${parseInt(parts[2])}`;
  return date;
}

function parsePercent(label: string): number {
  const m = label.match(/(\d+)/);
  return m ? parseInt(m[1]) : 50;
}

function EmptyState({ icon, title, desc }: { icon: ReactNode; title: string; desc: string }) {
  return (
    <div className="flex-1 flex flex-col items-center justify-center text-center py-20">
      {icon}
      <p className="text-base font-medium mb-1">{title}</p>
      <p style={{ fontSize: 13, color: "var(--color-text-secondary)" }}>{desc}</p>
    </div>
  );
}

function LoadingSkeleton() {
  return (
    <div className="space-y-3 mt-6">
      <div className="h-20 bg-stone-100 dark:bg-white/[0.06] rounded-xl animate-pulse" />
      <div className="h-48 bg-stone-100 dark:bg-white/[0.06] rounded-xl animate-pulse" />
      <div className="h-32 bg-stone-100 dark:bg-white/[0.06] rounded-xl animate-pulse" />
    </div>
  );
}

function AnalyzingSkeleton() {
  const { t } = useTranslation("digest");
  return (
    <div className="space-y-3 mt-6">
      <div className="h-20 bg-stone-100 dark:bg-white/[0.06] rounded-xl animate-pulse" />
      <div className="h-48 bg-stone-100 dark:bg-white/[0.06] rounded-xl animate-pulse" />
      <div className="text-center py-4">
        <RefreshCw size={16} className="animate-spin text-stone-400 mx-auto mb-2" />
        <p className="text-stone-400" style={{ fontSize: 13 }}>{t("radar.analyzing")}</p>
      </div>
    </div>
  );
}
