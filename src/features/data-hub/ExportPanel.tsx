import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useDataHubStore } from "../../stores/dataHubStore";
import {
  exportAll,
  type ImportSummary,
  importDirectory,
  importFiles,
  openExportDir,
  pickImportDirectory,
  pickImportFiles,
} from "../../services/dataHubService";
import { compileContentsToWiki } from "../../services/wikiService";

interface ExportPanelProps {
  onClose: () => void;
}

export function ExportPanel({ onClose }: ExportPanelProps) {
  const { t } = useTranslation("dataHub");
  const exportDir = useDataHubStore((s) => s.exportDir);
  const loadExportDir = useDataHubStore((s) => s.loadExportDir);
  const loadDateList = useDataHubStore((s) => s.loadDateList);
  const selectedDate = useDataHubStore((s) => s.selectedDate);
  const selectDate = useDataHubStore((s) => s.selectDate);
  const [isExporting, setIsExporting] = useState(false);
  const [isImporting, setIsImporting] = useState(false);
  const [isAddingToKnowledge, setIsAddingToKnowledge] = useState(false);
  const [exportResult, setExportResult] = useState<number | null>(null);
  const [importResult, setImportResult] = useState<ImportSummary | null>(null);
  const [knowledgeStatus, setKnowledgeStatus] = useState<"idle" | "done">("idle");

  useEffect(() => {
    loadExportDir();
  }, [loadExportDir]);

  const handleExportAll = async () => {
    setIsExporting(true);
    setExportResult(null);
    try {
      const count = await exportAll();
      setExportResult(count);
    } catch (e) {
      console.error("Failed to export all:", e);
    } finally {
      setIsExporting(false);
    }
  };

  const handleOpenFolder = async () => {
    try {
      await openExportDir();
    } catch (e) {
      console.error("Failed to open export dir:", e);
    }
  };

  const refreshImportedView = async () => {
    await loadDateList();
    if (selectedDate) {
      await selectDate(selectedDate);
    }
  };

  const handleImportFiles = async () => {
    setIsImporting(true);
    setImportResult(null);
    setKnowledgeStatus("idle");
    try {
      const paths = await pickImportFiles();
      if (paths.length === 0) return;
      const result = await importFiles(paths);
      setImportResult(result);
      await refreshImportedView();
    } catch (e) {
      console.error("Failed to import files:", e);
    } finally {
      setIsImporting(false);
    }
  };

  const handleImportDirectory = async () => {
    setIsImporting(true);
    setImportResult(null);
    setKnowledgeStatus("idle");
    try {
      const path = await pickImportDirectory();
      if (!path) return;
      const result = await importDirectory(path);
      setImportResult(result);
      await refreshImportedView();
    } catch (e) {
      console.error("Failed to import directory:", e);
    } finally {
      setIsImporting(false);
    }
  };

  const handleAddImportedToKnowledge = async () => {
    const importedIds = importResult?.imported_ids ?? [];
    if (importedIds.length === 0) return;
    setIsAddingToKnowledge(true);
    try {
      await compileContentsToWiki(importedIds);
      setKnowledgeStatus("done");
    } catch (e) {
      console.error("Failed to add imported content to knowledge:", e);
      setKnowledgeStatus("idle");
    } finally {
      setIsAddingToKnowledge(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/20 dark:bg-black/40"
        onClick={onClose}
      />

      {/* Panel */}
      <div className="relative glass-elevated rounded-2xl w-full max-w-sm mx-4 overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-white/30 dark:border-white/[0.06]">
          <h3 className="text-base font-semibold text-gray-800 dark:text-gray-100 flex items-center gap-2">
            <span>📤</span>
            {t("export.title")}
          </h3>
          <button
            onClick={onClose}
            className="w-7 h-7 flex items-center justify-center rounded-lg
                       text-gray-400 dark:text-slate-500 hover:bg-white/50 dark:hover:bg-white/[0.08]
                       transition-colors text-lg"
          >
            &times;
          </button>
        </div>

        {/* Content */}
        <div className="p-5 space-y-4">
          <div className="rounded-xl border border-white/50 dark:border-white/[0.06] bg-white/30 dark:bg-white/[0.03] p-4 space-y-3">
            <div>
              <div className="text-sm font-medium text-gray-700 dark:text-gray-300">
                {t("import.title")}
              </div>
              <div className="text-xs text-gray-400 dark:text-slate-500 mt-1 leading-relaxed">
                {t("import.description")}
              </div>
            </div>
            <div className="grid grid-cols-1 gap-2">
              <button
                onClick={handleImportFiles}
                disabled={isImporting}
                className="w-full flex items-center justify-center gap-2 px-4 py-2.5 text-sm font-medium rounded-xl border
                           bg-white/50 dark:bg-white/[0.04] border-white/60 dark:border-white/[0.08]
                           text-gray-600 dark:text-slate-300
                           hover:bg-white/80 dark:hover:bg-white/[0.08]
                           disabled:opacity-50 disabled:cursor-not-allowed
                           transition-all duration-150"
              >
                <span>{isImporting ? "⏳" : "📄"}</span>
                <span>{t("import.files")}</span>
              </button>
              <button
                onClick={handleImportDirectory}
                disabled={isImporting}
                className="w-full flex items-center justify-center gap-2 px-4 py-2.5 text-sm font-medium rounded-xl border
                           bg-white/50 dark:bg-white/[0.04] border-white/60 dark:border-white/[0.08]
                           text-gray-600 dark:text-slate-300
                           hover:bg-white/80 dark:hover:bg-white/[0.08]
                           disabled:opacity-50 disabled:cursor-not-allowed
                           transition-all duration-150"
              >
                <span>{isImporting ? "⏳" : "📚"}</span>
                <span>{t("import.folder")}</span>
              </button>
            </div>
            {importResult && (
              <div className="space-y-2 px-3 py-2 rounded-xl bg-green-500/10 dark:bg-green-500/15 border border-green-300/40 dark:border-green-500/20">
                <p className="text-xs text-green-700 dark:text-green-400 text-center">
                  {t("import.result", {
                    imported: importResult.imported,
                    skipped: importResult.skipped,
                    failed: importResult.failed,
                  })}
                </p>
                {(importResult.imported_ids?.length ?? 0) > 0 && (
                  <button
                    onClick={handleAddImportedToKnowledge}
                    disabled={isAddingToKnowledge}
                    className="w-full flex items-center justify-center gap-2 px-3 py-2 text-xs font-medium rounded-lg border
                               border-green-300/40 dark:border-green-500/20
                               bg-white/60 dark:bg-white/[0.04]
                               text-green-700 dark:text-green-300
                               hover:bg-white/80 dark:hover:bg-white/[0.08]
                               disabled:opacity-50 disabled:cursor-not-allowed
                               transition-all duration-150"
                  >
                    <span>{isAddingToKnowledge ? "⏳" : "🧠"}</span>
                    <span>
                      {isAddingToKnowledge
                        ? t("import.addingToKnowledge")
                        : knowledgeStatus === "done"
                          ? t("import.addedToKnowledge")
                          : t("import.addToKnowledge")}
                    </span>
                  </button>
                )}
              </div>
            )}
          </div>

          {/* Export directory */}
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1.5">
              {t("export.exportDir")}
            </label>
            <div
              className="px-3 py-2 text-xs text-gray-500 dark:text-slate-400 bg-white/40 dark:bg-white/[0.04] rounded-xl
                         border border-white/50 dark:border-white/[0.06] font-mono break-all"
            >
              {exportDir || t("export.notSet")}
            </div>
          </div>

          {/* Export all button */}
          <button
            onClick={handleExportAll}
            disabled={isExporting}
            className="w-full flex items-center justify-center gap-2 px-4 py-2.5 text-sm font-medium rounded-xl border
                       bg-orange-500/10 dark:bg-orange-500/15
                       border-orange-300/60 dark:border-orange-500/30
                       text-orange-700 dark:text-orange-400
                       hover:bg-orange-500/15 dark:hover:bg-orange-500/20
                       disabled:opacity-50 disabled:cursor-not-allowed
                       transition-all duration-150"
          >
            {isExporting ? (
              <>
                <span className="animate-spin">⏳</span>
                <span>{t("export.exporting")}</span>
              </>
            ) : (
              <>
                <span>📦</span>
                <span>{t("export.exportAll")}</span>
              </>
            )}
          </button>

          {/* Export result */}
          {exportResult !== null && (
            <div className="px-3 py-2 rounded-xl bg-green-500/10 dark:bg-green-500/15 border border-green-300/40 dark:border-green-500/20">
              <p className="text-xs text-green-700 dark:text-green-400 text-center">
                {t("export.exportedFiles", { count: exportResult })}
              </p>
            </div>
          )}

          {/* Open in Finder */}
          <button
            onClick={handleOpenFolder}
            className="w-full flex items-center justify-center gap-2 px-4 py-2.5 text-sm font-medium rounded-xl border
                       bg-white/50 dark:bg-white/[0.04] border-white/60 dark:border-white/[0.08]
                       text-gray-600 dark:text-slate-300
                       hover:bg-white/80 dark:hover:bg-white/[0.08]
                       transition-all duration-150"
          >
            <span>📁</span>
            <span>{t("export.openInFinder")}</span>
          </button>
        </div>
      </div>
    </div>
  );
}
