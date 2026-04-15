import { useEffect, useMemo, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { BookOpen, CheckCircle2, ExternalLink, HardDrive, Target } from "lucide-react";
import { getSettings, updateSetting } from "../../services/settingsService";
import { syncLocalRawDirectory } from "../../services/dataHubService";
import { syncLocalWiki } from "../../services/wikiService";
import {
  getWorkspacePaths,
  initializeWorkspaceRoot,
  openWorkspaceRoot,
  type WorkspacePaths,
} from "../../services/workspaceService";
import {
  useSettingsStore,
  type WorkspaceStorageMode,
} from "../../stores/settingsStore";

interface ModeOption {
  value: WorkspaceStorageMode;
  titleKey: string;
  descKey: string;
  badgeKey: string;
}

const MODE_OPTIONS: ModeOption[] = [
  {
    value: "managed",
    titleKey: "workspaceSetup.modeManagedTitle",
    descKey: "workspaceSetup.modeManagedDesc",
    badgeKey: "workspaceSetup.modeManagedBadge",
  },
  {
    value: "connected",
    titleKey: "workspaceSetup.modeConnectedTitle",
    descKey: "workspaceSetup.modeConnectedDesc",
    badgeKey: "workspaceSetup.modeConnectedBadge",
  },
  {
    value: "hybrid",
    titleKey: "workspaceSetup.modeHybridTitle",
    descKey: "workspaceSetup.modeHybridDesc",
    badgeKey: "workspaceSetup.modeHybridBadge",
  },
];

export function WorkspaceSetupView() {
  const { t } = useTranslation("settings");
  const completeWorkspaceSetup = useSettingsStore((s) => s.completeWorkspaceSetup);
  const storageMode = useSettingsStore((s) => s.storageMode);

  const [selectedMode, setSelectedMode] = useState<WorkspaceStorageMode>(storageMode);
  const [localWikiPath, setLocalWikiPath] = useState("");
  const [localRawPath, setLocalRawPath] = useState("");
  const [workspaceRoot, setWorkspaceRoot] = useState("");
  const [workspacePaths, setWorkspacePaths] = useState<WorkspacePaths | null>(null);
  const [saving, setSaving] = useState(false);
  const [initializingWorkspace, setInitializingWorkspace] = useState(false);
  const [syncingRaw, setSyncingRaw] = useState(false);
  const [syncingWiki, setSyncingWiki] = useState(false);
  const [rawResult, setRawResult] = useState("");
  const [wikiResult, setWikiResult] = useState("");
  const [errorMessage, setErrorMessage] = useState("");

  useEffect(() => {
    getSettings()
      .then((settings) => {
        setLocalWikiPath(settings.wiki_local_source_path || "");
        setLocalRawPath(settings.wiki_raw_source_path || "");
        setWorkspaceRoot(settings.workspace_root || "");
      })
      .catch(() => {});
    getWorkspacePaths()
      .then((paths) => {
        setWorkspacePaths(paths);
        setWorkspaceRoot((current) => current || paths.root);
      })
      .catch(() => {});
  }, []);

  const needsExternalPaths = selectedMode === "connected";
  const canFinish = useMemo(() => {
    if (selectedMode === "managed" || selectedMode === "hybrid") return true;
    return Boolean(localWikiPath.trim() || localRawPath.trim());
  }, [localRawPath, localWikiPath, selectedMode]);

  const persistPaths = async () => {
    await Promise.all([
      updateSetting("wiki_local_source_path", localWikiPath.trim()),
      updateSetting("wiki_raw_source_path", localRawPath.trim()),
    ]);
  };

  const handleInitializeWorkspace = async () => {
    setInitializingWorkspace(true);
    setErrorMessage("");
    try {
      const paths = await initializeWorkspaceRoot(workspaceRoot.trim());
      setWorkspacePaths(paths);
      setWorkspaceRoot(paths.root);
    } catch (error) {
      setErrorMessage(t("workspaceSetup.workspaceInitFailed", { error: String(error) }));
    }
    setInitializingWorkspace(false);
  };

  const handleComplete = async (mode: WorkspaceStorageMode) => {
    setSaving(true);
    setErrorMessage("");
    try {
      const paths = await initializeWorkspaceRoot(workspaceRoot.trim());
      setWorkspacePaths(paths);
      setWorkspaceRoot(paths.root);
      await persistPaths();
      await completeWorkspaceSetup(mode);
    } catch (error) {
      setErrorMessage(t("workspaceSetup.completeFailed", { error: String(error) }));
    }
    setSaving(false);
  };

  const handleSyncRaw = async () => {
    const trimmed = localRawPath.trim();
    if (!trimmed) return;
    setSyncingRaw(true);
    setRawResult("");
    setErrorMessage("");
    try {
      await updateSetting("wiki_raw_source_path", trimmed);
      const result = await syncLocalRawDirectory(trimmed);
      setRawResult(
        t("workspaceSetup.rawSyncResult", {
          files: result.files_found,
          created: result.created,
          updated: result.updated,
          removed: result.removed,
        })
      );
    } catch (error) {
      setErrorMessage(t("workspaceSetup.rawSyncFailed", { error: String(error) }));
    }
    setSyncingRaw(false);
  };

  const handleSyncWiki = async () => {
    const trimmed = localWikiPath.trim();
    if (!trimmed) return;
    setSyncingWiki(true);
    setWikiResult("");
    setErrorMessage("");
    try {
      await updateSetting("wiki_local_source_path", trimmed);
      const result = await syncLocalWiki(trimmed);
      setWikiResult(
        t("workspaceSetup.wikiSyncResult", {
          pages: result.pages_found,
          created: result.created,
          updated: result.updated,
          removed: result.removed,
        })
      );
    } catch (error) {
      setErrorMessage(t("workspaceSetup.wikiSyncFailed", { error: String(error) }));
    }
    setSyncingWiki(false);
  };

  return (
    <div className="min-h-screen bg-[#FAFAF8] dark:bg-[#0C0A09] text-gray-900 dark:text-gray-100">
      <div className="mx-auto max-w-6xl px-6 py-10">
        <div className="max-w-3xl">
          <p className="text-xs font-semibold uppercase tracking-[0.18em] text-orange-500">
            OpenWiki Setup
          </p>
          <h1 className="mt-3 text-4xl font-semibold tracking-tight">
            {t("workspaceSetup.title")}
          </h1>
          <p className="mt-3 text-base leading-7 text-gray-600 dark:text-slate-300">
            {t("workspaceSetup.subtitle")}
          </p>
        </div>

        <div className="mt-8 grid gap-4 md:grid-cols-3">
          <StageCard
            icon={<HardDrive className="h-5 w-5 text-orange-500" />}
            title={t("workspaceSetup.libraryTitle")}
            description={t("workspaceSetup.libraryDesc")}
          />
          <StageCard
            icon={<BookOpen className="h-5 w-5 text-orange-500" />}
            title={t("workspaceSetup.knowledgeTitle")}
            description={t("workspaceSetup.knowledgeDesc")}
          />
          <StageCard
            icon={<Target className="h-5 w-5 text-orange-500" />}
            title={t("workspaceSetup.insightsTitle")}
            description={t("workspaceSetup.insightsDesc")}
          />
        </div>

        <div className="mt-10 grid gap-4 xl:grid-cols-[1.4fr_1fr]">
          <div className="rounded-2xl border border-orange-200/50 bg-white/80 p-6 shadow-sm dark:border-white/[0.08] dark:bg-white/[0.04]">
            <div className="flex items-center justify-between">
              <div>
                <h2 className="text-lg font-semibold">{t("workspaceSetup.modeTitle")}</h2>
                <p className="mt-1 text-sm text-gray-500 dark:text-slate-400">
                  {t("workspaceSetup.modeDesc")}
                </p>
              </div>
              <button
                onClick={() => invoke("open_data_folder").catch((e) => console.error("open_data_folder failed:", e))}
                className="inline-flex items-center gap-2 rounded-lg border border-orange-200/50 px-3 py-2 text-xs font-medium text-orange-500 transition hover:bg-orange-50 dark:border-orange-400/20 dark:hover:bg-orange-500/10"
              >
                <ExternalLink className="h-3.5 w-3.5" />
                {t("workspaceSetup.openManagedStorage")}
              </button>
            </div>

            <div className="mt-5 grid gap-3 md:grid-cols-3">
              {MODE_OPTIONS.map((option) => {
                const active = option.value === selectedMode;
                return (
                  <button
                    key={option.value}
                    onClick={() => setSelectedMode(option.value)}
                    className={`rounded-xl border px-4 py-4 text-left transition ${
                      active
                        ? "border-orange-400 bg-orange-50 text-orange-600 shadow-sm dark:bg-orange-500/10"
                        : "border-gray-200 bg-white/70 text-gray-700 hover:border-orange-300 dark:border-white/[0.08] dark:bg-white/[0.03] dark:text-slate-200"
                    }`}
                  >
                    <div className="flex items-center justify-between gap-3">
                      <span className="text-sm font-semibold">{t(option.titleKey)}</span>
                      {active ? <CheckCircle2 className="h-4 w-4" /> : <span className="text-[11px] text-gray-400 dark:text-slate-500">{t(option.badgeKey)}</span>}
                    </div>
                    <p className="mt-2 text-xs leading-5 text-gray-500 dark:text-slate-400">
                      {t(option.descKey)}
                    </p>
                  </button>
                );
              })}
            </div>

            <div className="mt-6 rounded-xl border border-dashed border-orange-200/70 bg-orange-50/50 p-4 dark:border-orange-400/20 dark:bg-orange-500/5">
              <p className="text-sm font-medium text-gray-800 dark:text-slate-100">
                {t("workspaceSetup.mappingTitle")}
              </p>
              <p className="mt-1 text-xs leading-5 text-gray-500 dark:text-slate-400">
                {t("workspaceSetup.mappingDesc")}
              </p>
            </div>

            <div className="mt-6 rounded-xl border border-gray-200/70 bg-white/70 p-4 dark:border-white/[0.08] dark:bg-white/[0.02]">
              <p className="text-sm font-medium">{t("workspaceSetup.workspaceRootTitle")}</p>
              <p className="mt-1 text-xs leading-5 text-gray-500 dark:text-slate-400">
                {t("workspaceSetup.workspaceRootDesc")}
              </p>
              <input
                value={workspaceRoot}
                onChange={(e) => setWorkspaceRoot(e.target.value)}
                placeholder="~/Documents/OpenWiki Workspace"
                className="mt-3 w-full rounded-lg border border-gray-200/80 bg-white px-3 py-2 text-sm text-gray-700 outline-none transition focus:border-orange-400 dark:border-white/[0.08] dark:bg-white/[0.04] dark:text-slate-200"
              />
              <div className="mt-3 flex flex-wrap gap-2">
                <button
                  onClick={handleInitializeWorkspace}
                  disabled={initializingWorkspace}
                  className="rounded-lg border border-orange-200/60 px-3 py-2 text-xs font-medium text-orange-500 transition hover:bg-orange-50 disabled:opacity-40 dark:border-orange-400/20 dark:hover:bg-orange-500/10"
                >
                  {initializingWorkspace
                    ? t("workspaceSetup.initializingWorkspace")
                    : t("workspaceSetup.initializeWorkspace")}
                </button>
                <button
                  onClick={() => openWorkspaceRoot().catch((error) => setErrorMessage(String(error)))}
                  className="rounded-lg border border-orange-200/60 px-3 py-2 text-xs font-medium text-orange-500 transition hover:bg-orange-50 dark:border-orange-400/20 dark:hover:bg-orange-500/10"
                >
                  {t("workspaceSetup.openWorkspace")}
                </button>
              </div>
              {workspacePaths && (
                <div className="mt-3 rounded-lg bg-orange-50/70 px-3 py-3 text-[11px] leading-5 text-gray-600 dark:bg-orange-500/5 dark:text-slate-300">
                  <p>{t("workspaceSetup.workspaceMappedTo", { path: workspacePaths.inbox })}</p>
                  <p>{t("workspaceSetup.workspaceMappedRaw", { path: workspacePaths.raw })}</p>
                  <p>{t("workspaceSetup.workspaceMappedWiki", { path: workspacePaths.wiki_root })}</p>
                  <p>{t("workspaceSetup.workspaceMappedInsights", { path: workspacePaths.insights })}</p>
                </div>
              )}
            </div>

            <div className="mt-6 grid gap-4 md:grid-cols-2">
              <div className="rounded-xl border border-gray-200/70 bg-white/70 p-4 dark:border-white/[0.08] dark:bg-white/[0.02]">
                <p className="text-sm font-medium">{t("workspaceSetup.localRawTitle")}</p>
                <p className="mt-1 text-xs leading-5 text-gray-500 dark:text-slate-400">
                  {t("workspaceSetup.localRawDesc")}
                </p>
                <input
                  value={localRawPath}
                  onChange={(e) => setLocalRawPath(e.target.value)}
                  placeholder="/path/to/raw"
                  className="mt-3 w-full rounded-lg border border-gray-200/80 bg-white px-3 py-2 text-sm text-gray-700 outline-none transition focus:border-orange-400 dark:border-white/[0.08] dark:bg-white/[0.04] dark:text-slate-200"
                />
                <button
                  onClick={handleSyncRaw}
                  disabled={syncingRaw || !localRawPath.trim()}
                  className="mt-3 rounded-lg border border-orange-200/60 px-3 py-2 text-xs font-medium text-orange-500 transition hover:bg-orange-50 disabled:opacity-40 dark:border-orange-400/20 dark:hover:bg-orange-500/10"
                >
                  {syncingRaw ? t("workspaceSetup.syncingRaw") : t("workspaceSetup.syncRaw")}
                </button>
                {rawResult && <p className="mt-2 text-xs leading-5 text-gray-500 dark:text-slate-400">{rawResult}</p>}
              </div>

              <div className="rounded-xl border border-gray-200/70 bg-white/70 p-4 dark:border-white/[0.08] dark:bg-white/[0.02]">
                <p className="text-sm font-medium">{t("workspaceSetup.localWikiTitle")}</p>
                <p className="mt-1 text-xs leading-5 text-gray-500 dark:text-slate-400">
                  {t("workspaceSetup.localWikiDesc")}
                </p>
                <input
                  value={localWikiPath}
                  onChange={(e) => setLocalWikiPath(e.target.value)}
                  placeholder="/path/to/知识库"
                  className="mt-3 w-full rounded-lg border border-gray-200/80 bg-white px-3 py-2 text-sm text-gray-700 outline-none transition focus:border-orange-400 dark:border-white/[0.08] dark:bg-white/[0.04] dark:text-slate-200"
                />
                <button
                  onClick={handleSyncWiki}
                  disabled={syncingWiki || !localWikiPath.trim()}
                  className="mt-3 rounded-lg border border-orange-200/60 px-3 py-2 text-xs font-medium text-orange-500 transition hover:bg-orange-50 disabled:opacity-40 dark:border-orange-400/20 dark:hover:bg-orange-500/10"
                >
                  {syncingWiki ? t("workspaceSetup.syncingWiki") : t("workspaceSetup.syncWiki")}
                </button>
                {wikiResult && <p className="mt-2 text-xs leading-5 text-gray-500 dark:text-slate-400">{wikiResult}</p>}
              </div>
            </div>

            {needsExternalPaths && !canFinish && (
              <p className="mt-4 text-xs text-orange-500">
                {t("workspaceSetup.connectedNeedsPath")}
              </p>
            )}

            {errorMessage && (
              <div className="mt-4 rounded-lg border border-red-200/70 bg-red-50 px-3 py-2 text-xs text-red-600 dark:border-red-500/20 dark:bg-red-500/10 dark:text-red-300">
                {errorMessage}
              </div>
            )}
          </div>

          <div className="rounded-2xl border border-gray-200/70 bg-white/80 p-6 shadow-sm dark:border-white/[0.08] dark:bg-white/[0.04]">
            <h2 className="text-lg font-semibold">{t("workspaceSetup.summaryTitle")}</h2>
            <p className="mt-2 text-sm leading-6 text-gray-500 dark:text-slate-400">
              {t("workspaceSetup.summaryDesc")}
            </p>

            <div className="mt-5 space-y-3">
              <SummaryRow title={t("workspaceSetup.summaryStorage")} body={t(`workspaceSetup.summaryStorage${selectedMode[0].toUpperCase()}${selectedMode.slice(1)}`)} />
              <SummaryRow title={t("workspaceSetup.summaryRaw")} body={localRawPath.trim() || t("workspaceSetup.notConfigured")} />
              <SummaryRow title={t("workspaceSetup.summaryWiki")} body={localWikiPath.trim() || t("workspaceSetup.notConfigured")} />
            </div>

            <div className="mt-6 rounded-xl bg-gray-50 px-4 py-4 text-xs leading-6 text-gray-500 dark:bg-white/[0.03] dark:text-slate-400">
              {t("workspaceSetup.laterHint")}
            </div>

            <div className="mt-6 flex flex-col gap-3">
              <button
                onClick={() => handleComplete(selectedMode)}
                disabled={saving || !canFinish}
                className="rounded-lg bg-orange-500 px-4 py-3 text-sm font-medium text-white transition hover:bg-orange-600 disabled:opacity-40"
              >
                {saving ? t("workspaceSetup.finishing") : t("workspaceSetup.finish")}
              </button>
              <button
                onClick={() => handleComplete("managed")}
                disabled={saving}
                className="rounded-lg border border-gray-200/70 px-4 py-3 text-sm font-medium text-gray-600 transition hover:border-orange-300 hover:text-orange-500 disabled:opacity-40 dark:border-white/[0.08] dark:text-slate-300"
              >
                {t("workspaceSetup.skipForNow")}
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function StageCard({
  icon,
  title,
  description,
}: {
  icon: ReactNode;
  title: string;
  description: string;
}) {
  return (
    <div className="rounded-2xl border border-gray-200/70 bg-white/80 p-5 shadow-sm dark:border-white/[0.08] dark:bg-white/[0.04]">
      <div className="flex items-center gap-3">
        <div className="rounded-lg bg-orange-50 p-2 dark:bg-orange-500/10">{icon}</div>
        <h2 className="text-base font-semibold">{title}</h2>
      </div>
      <p className="mt-3 text-sm leading-6 text-gray-500 dark:text-slate-400">{description}</p>
    </div>
  );
}

function SummaryRow({ title, body }: { title: string; body: string }) {
  return (
    <div className="rounded-xl border border-gray-200/70 px-4 py-3 dark:border-white/[0.08]">
      <p className="text-xs font-medium uppercase tracking-[0.14em] text-gray-400 dark:text-slate-500">
        {title}
      </p>
      <p className="mt-1 text-sm leading-6 text-gray-700 dark:text-slate-200">
        {body}
      </p>
    </div>
  );
}
