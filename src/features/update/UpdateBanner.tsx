import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { open as openExternal } from "@tauri-apps/plugin-shell";
import { useTranslation } from "react-i18next";
import { CheckCircle2, Download, Loader2, X } from "lucide-react";
import { motion } from "framer-motion";
import { type UpdateInfo } from "../../services/updateService";
import { useUpdateStore } from "../../stores/updateStore";

/**
 * Restart confirmation for a downloaded update, in the corner of any page.
 *
 * The background GitHub Releases check (`src-tauri/src/update/mod.rs`) reports a newer version
 * and the update store downloads it quietly. Only once the package is ready (or the download
 * failed) does this dialog ask the user. Settings → App updates shows the same state inline, so
 * the dialog stays away while that page is open.
 */
export function UpdateBanner() {
  const { t } = useTranslation("update");
  const info = useUpdateStore((s) => s.info);
  const phase = useUpdateStore((s) => s.phase);
  const failure = useUpdateStore((s) => s.failure);
  const error = useUpdateStore((s) => s.error);
  const inlineVisible = useUpdateStore((s) => s.inlineVisible);
  const dialogDismissed = useUpdateStore((s) => s.dialogDismissed);
  const prepare = useUpdateStore((s) => s.prepare);
  const install = useUpdateStore((s) => s.install);
  const retry = useUpdateStore((s) => s.retry);
  const dismissDialog = useUpdateStore((s) => s.dismissDialog);

  useEffect(() => {
    const unlisten = listen<UpdateInfo>("update-available", (event) => prepare(event.payload));
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [prepare]);

  if (!info || inlineVisible || dialogDismissed || (phase !== "ready" && phase !== "installing" && phase !== "failed")) {
    return null;
  }

  const installing = phase === "installing";
  const failed = phase === "failed";

  const handleViewNotes = async () => {
    try {
      await openExternal(info.url);
    } catch (err) {
      console.error("[update] failed to open release page:", err);
    }
  };

  const title = failed ? t("dialog.failedTitle") : t("dialog.title", { version: info.version });

  const description = failed
    ? t(failure === "install" ? "dialog.installFailedBody" : "dialog.downloadFailedBody", { error })
    : t("dialog.body", { version: info.version });

  const primaryLabel = installing
    ? t("dialog.installing")
    : failed
    ? t("dialog.retryDownload")
    : t("dialog.install");

  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.18, ease: "easeOut" }}
      className="fixed bottom-4 left-4 right-4 z-[100] flex justify-center
                 pointer-events-none
                 sm:bottom-5 sm:left-auto sm:right-5 sm:justify-end"
      role="dialog"
      aria-live="polite"
      aria-labelledby="update-ready-title"
      aria-describedby="update-ready-description"
    >
      <div
        className="relative w-full max-w-[420px] rounded-2xl pointer-events-auto
                   border border-stone-200/70 dark:border-white/[0.08]
                   bg-white text-stone-900 dark:bg-stone-900 dark:text-stone-50
                   shadow-[0_24px_70px_rgba(28,25,23,0.24)]
                   p-7"
      >
        <button
          onClick={dismissDialog}
          disabled={installing}
          aria-label={t("dialog.close")}
          className="absolute right-4 top-4 rounded-lg p-1.5 text-stone-400
                     transition-colors hover:bg-stone-100 hover:text-stone-700
                     disabled:cursor-not-allowed disabled:opacity-40
                     dark:text-stone-500 dark:hover:bg-white/[0.06] dark:hover:text-stone-200"
        >
          <X className="h-4 w-4" />
        </button>

        <div
          className="mb-5 flex h-12 w-12 items-center justify-center rounded-xl
                     bg-orange-50 text-orange-600 dark:bg-orange-500/15 dark:text-orange-300"
        >
          {failed ? (
            <Download className="h-6 w-6" strokeWidth={2.2} />
          ) : (
            <CheckCircle2 className="h-6 w-6" strokeWidth={2.2} />
          )}
        </div>

        <h2
          id="update-ready-title"
          className="mb-2 text-xl font-semibold tracking-normal text-stone-900 dark:text-stone-50"
        >
          {title}
        </h2>

        <p
          id="update-ready-description"
          className="mb-6 text-sm leading-6 text-stone-600 dark:text-stone-300"
        >
          {description}
        </p>

        <div className="flex gap-2.5">
          <button
            onClick={dismissDialog}
            disabled={installing}
            className="flex-1 rounded-lg border border-stone-200 bg-white py-2.5
                       text-sm font-medium text-stone-600 transition-colors
                       hover:bg-stone-50 disabled:cursor-not-allowed disabled:opacity-50
                       dark:border-white/[0.08] dark:bg-white/[0.03]
                       dark:text-stone-300 dark:hover:bg-white/[0.06]"
          >
            {t("dialog.later")}
          </button>

          <button
            onClick={failed ? retry : () => void install()}
            disabled={installing}
            className="flex-1 rounded-lg bg-orange-500 py-2.5 text-sm font-semibold
                       text-white transition-colors hover:bg-orange-600
                       disabled:cursor-wait disabled:opacity-75
                       flex items-center justify-center gap-2"
          >
            {installing && <Loader2 className="h-4 w-4 animate-spin" />}
            {primaryLabel}
          </button>
        </div>

        <button
          onClick={handleViewNotes}
          disabled={installing}
          className="mt-3 w-full rounded-lg py-2 text-xs font-medium
                     text-stone-500 transition-colors hover:bg-stone-50 hover:text-orange-600
                     disabled:cursor-not-allowed disabled:opacity-50
                     dark:text-stone-400 dark:hover:bg-white/[0.04] dark:hover:text-orange-300"
        >
          {failed ? t("dialog.downloadFallback") : t("dialog.view")}
        </button>
      </div>
    </motion.div>
  );
}
