import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import {
  getDataLocation,
  pickDataFolder,
  moveDataDir,
  removePreviousData,
  openPreviousDataFolder,
  type DataLocationInfo,
  type PickedDataFolder,
} from "../../services/dataLocationService";

const PRIMARY_BUTTON =
  "px-4 py-1.5 text-sm font-medium rounded-lg border text-orange-600 dark:text-orange-400 border-orange-200/50 dark:border-orange-500/20 bg-orange-50/50 dark:bg-orange-500/[0.06] hover:bg-orange-100/50 dark:hover:bg-orange-500/[0.12] disabled:opacity-50 transition-colors";
const SECONDARY_BUTTON =
  "px-4 py-1.5 text-sm font-medium rounded-lg border text-gray-600 dark:text-gray-300 border-gray-200/50 dark:border-white/[0.08] bg-white/40 dark:bg-white/[0.04] hover:bg-white/70 dark:hover:bg-white/[0.08] disabled:opacity-50 transition-colors";
const DANGER_BUTTON =
  "px-4 py-1.5 text-sm font-medium rounded-lg border text-red-600 dark:text-red-400 border-red-200/60 dark:border-red-500/20 bg-red-50/50 dark:bg-red-500/[0.06] hover:bg-red-100/50 dark:hover:bg-red-500/[0.12] disabled:opacity-50 transition-colors";

type MovePhase = "idle" | "picking" | "confirm" | "moving";

/** Where OpenWiki keeps its data, and moving it to another folder or drive (issue #17). */
export function DataLocationSection() {
  const { t } = useTranslation("settings");
  const [info, setInfo] = useState<DataLocationInfo | null>(null);
  const [phase, setPhase] = useState<MovePhase>("idle");
  const [picked, setPicked] = useState<PickedDataFolder | null>(null);
  const [progress, setProgress] = useState({ copied: 0, total: 0 });
  const [error, setError] = useState("");
  const [deletePhase, setDeletePhase] = useState<"idle" | "confirm" | "deleting">("idle");

  const describeError = useCallback(
    (e: unknown) => {
      const raw = typeof e === "string" ? e : String(e);
      const code = raw.split(":")[0].trim();
      return t(`storage.moveErrors.${code}`, { defaultValue: raw });
    },
    [t],
  );

  const refresh = useCallback(() => {
    getDataLocation()
      .then(setInfo)
      .catch((e) => console.error("get_data_location failed:", e));
  }, []);

  useEffect(() => {
    refresh();
    const unlisten = listen<{ copied: number; total: number }>("data-move-progress", (event) =>
      setProgress(event.payload),
    );
    return () => {
      unlisten.then((stop) => stop());
    };
  }, [refresh]);

  const handlePick = async () => {
    setError("");
    setPhase("picking");
    try {
      const folder = await pickDataFolder();
      if (folder) {
        setPicked(folder);
        setPhase("confirm");
      } else {
        setPhase("idle");
      }
    } catch (e) {
      setError(describeError(e));
      setPhase("idle");
    }
  };

  const handleMove = async () => {
    if (!picked) return;
    setError("");
    setProgress({ copied: 0, total: 0 });
    setPhase("moving");
    try {
      // Only comes back if something went wrong: on success OpenWiki restarts.
      await moveDataDir(picked.picked);
    } catch (e) {
      setError(t("storage.moveFailed", { error: describeError(e) }));
      setPhase("idle");
      setPicked(null);
    }
  };

  const handleDeletePrevious = async () => {
    setDeletePhase("deleting");
    setError("");
    try {
      await removePreviousData();
      refresh();
    } catch (e) {
      setError(t("storage.deleteFailed", { error: describeError(e) }));
    } finally {
      setDeletePhase("idle");
    }
  };

  const sizeText = (mb: number) => (mb < 10 ? mb.toFixed(1) : Math.round(mb).toString());
  const percent = progress.total > 0 ? Math.round((progress.copied / progress.total) * 100) : 0;

  return (
    <div className="glass rounded-2xl divide-y divide-gray-100/50 dark:divide-white/[0.06] mt-3">
      <div className="p-4">
        <div className="flex items-center gap-2 mb-1">
          <span className="text-sm font-medium text-gray-700 dark:text-gray-300">{t("storage.dataLocation")}</span>
          {info?.is_default && (
            <span className="text-[11px] px-1.5 py-0.5 rounded bg-gray-100 dark:bg-white/[0.06] text-gray-500 dark:text-slate-400">
              {t("storage.defaultTag")}
            </span>
          )}
        </div>
        <div className="text-xs font-mono text-gray-500 dark:text-slate-400 break-all mb-2">{info?.path ?? "…"}</div>
        <div className="text-xs text-gray-400 dark:text-slate-500 mb-3">
          {t("storage.dataLocationDesc", { size: sizeText(info?.size_mb ?? 0) })}
        </div>
        {(phase === "idle" || phase === "picking") && (
          <button onClick={handlePick} disabled={phase === "picking" || !info} className={`w-full ${SECONDARY_BUTTON}`}>
            {phase === "picking" ? t("storage.picking") : t("storage.changeLocation")}
          </button>
        )}

        {phase === "confirm" && picked && (
          <div className="rounded-xl border border-orange-200/50 dark:border-orange-500/20 bg-orange-50/30 dark:bg-orange-500/[0.04] p-3">
            <div className="text-sm text-gray-700 dark:text-gray-200 mb-1">
              {t("storage.confirmTitle", { size: sizeText(info?.size_mb ?? 0) })}
            </div>
            <div className="text-xs font-mono text-gray-600 dark:text-slate-300 break-all mb-2">{picked.target}</div>
            <div className="text-xs text-gray-500 dark:text-slate-400 mb-1">{t("storage.confirmBody")}</div>
            <div className="text-xs text-gray-500 dark:text-slate-400 mb-3">{t("storage.confirmExternal")}</div>
            <div className="flex gap-2">
              <button onClick={handleMove} className={PRIMARY_BUTTON}>
                {t("storage.startMove")}
              </button>
              <button
                onClick={() => {
                  setPicked(null);
                  setPhase("idle");
                }}
                className={SECONDARY_BUTTON}
              >
                {t("storage.cancel")}
              </button>
            </div>
          </div>
        )}

        {phase === "moving" && (
          <div>
            <div className="text-sm text-gray-700 dark:text-gray-200 mb-2">
              {t("storage.moving", { copied: progress.copied, total: progress.total || "…" })}
            </div>
            <div className="h-1.5 rounded-full bg-gray-100 dark:bg-white/[0.06] overflow-hidden mb-2">
              <div className="h-full bg-orange-400 transition-all" style={{ width: `${percent}%` }} />
            </div>
            <div className="text-xs text-gray-400 dark:text-slate-500">{t("storage.movingHint")}</div>
          </div>
        )}

        {error && <p className="mt-2 text-xs text-red-500 dark:text-red-400">{error}</p>}
      </div>

      {info?.previous && phase !== "moving" && (
        <div className="p-4">
          <div className="text-sm text-gray-700 dark:text-gray-300 mb-1">
            {t("storage.previousTitle", { size: sizeText(info.previous.size_mb) })}
          </div>
          <div className="text-xs font-mono text-gray-500 dark:text-slate-400 break-all mb-2">{info.previous.path}</div>
          <div className="text-xs text-gray-400 dark:text-slate-500 mb-3">{t("storage.previousDesc")}</div>
          <div className="flex gap-2 flex-wrap">
            <button
              onClick={() => openPreviousDataFolder().catch((e) => setError(describeError(e)))}
              className={SECONDARY_BUTTON}
            >
              {t("storage.openPrevious")}
            </button>
            {deletePhase === "idle" ? (
              <button onClick={() => setDeletePhase("confirm")} className={DANGER_BUTTON}>
                {t("storage.deletePrevious")}
              </button>
            ) : (
              <>
                <button onClick={handleDeletePrevious} disabled={deletePhase === "deleting"} className={DANGER_BUTTON}>
                  {deletePhase === "deleting" ? t("storage.deleting") : t("storage.confirmDelete")}
                </button>
                <button
                  onClick={() => setDeletePhase("idle")}
                  disabled={deletePhase === "deleting"}
                  className={SECONDARY_BUTTON}
                >
                  {t("storage.cancel")}
                </button>
              </>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
