import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { Check, Info, Play, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useSettingsStore } from "../../stores/settingsStore";
import { FootballDemo } from "./footballDemo";
import "./whats-new.css";

const IS_MAC = typeof navigator !== "undefined" && /Macintosh/i.test(navigator.userAgent);
const STEPS = [1, 2, 3] as const;
const FOCUS_RING = "focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-orange-500";

/**
 * One-time card in the main window after an update: a looping demo of the football bubble
 * and a button to turn it on. The backend decides whether it is due (get_whats_new) and opens
 * the main window for it; the card counts as seen as soon as it is on screen.
 */
export function WhatsNewDialog() {
  const { t } = useTranslation("update");
  const [id, setId] = useState<string | null>(null);
  const [version, setVersion] = useState("");
  const [autoCapture, setAutoCapture] = useState(false);
  const [closing, setClosing] = useState(false);
  const [enabled, setEnabled] = useState(false);
  const [step, setStep] = useState(0);
  const [poster, setPoster] = useState(false);
  const [toast, setToast] = useState(false);
  const dialogRef = useRef<HTMLDivElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const keycapRef = useRef<HTMLDivElement>(null);
  const cheerRef = useRef<HTMLDivElement>(null);
  const demoRef = useRef<FootballDemo | null>(null);

  useEffect(() => {
    let cancelled = false;
    const check = async () => {
      const pending = await invoke<string | null>("get_whats_new");
      if (!pending || cancelled) return;
      const [settings, appVersion] = await Promise.all([
        invoke<Record<string, string>>("get_settings").catch((): Record<string, string> => ({})),
        getVersion().catch(() => ""),
      ]);
      if (cancelled) return;
      setAutoCapture(settings.capture_mode === "auto");
      setVersion(appVersion);
      setId(pending);
      // Shown once: closing the window without choosing must not bring it back on every launch.
      invoke("mark_whats_new_seen", { id: pending }).catch((e) => console.error("[whats-new] mark seen failed:", e));
    };
    check().catch((e) => console.error("[whats-new] check failed:", e));
    return () => {
      cancelled = true;
    };
  }, []);

  // The demo runs only while the card is on screen.
  useEffect(() => {
    if (!id || closing) return;
    const canvas = canvasRef.current;
    const stage = stageRef.current;
    const keycap = keycapRef.current;
    const cheer = cheerRef.current;
    if (!canvas || !stage || !keycap || !cheer) return;
    const demo = new FootballDemo({ canvas, stage, keycap, cheer, onStep: setStep, onPoster: setPoster });
    demoRef.current = demo;
    demo.start();
    dialogRef.current?.focus({ preventScroll: true });
    return () => {
      demo.destroy();
      demoRef.current = null;
    };
  }, [id, closing]);

  const close = useCallback(() => {
    setClosing(true);
    window.setTimeout(() => setId(null), 160);
  }, []);

  const enable = useCallback(() => {
    if (enabled) return;
    setEnabled(true);
    const store = useSettingsStore.getState();
    store.setBubbleStyle("football");
    if (autoCapture) {
      // Auto capture never shows a reminder: switch to confirm, and still save copies nobody kicks.
      store.setCaptureMode("confirm");
      store.setDefaultAction("save");
    }
    window.setTimeout(() => {
      close();
      setToast(true);
      window.setTimeout(() => setToast(false), 2600);
    }, 700);
  }, [autoCapture, close, enabled]);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
      return;
    }
    const dialog = dialogRef.current;
    if (e.key !== "Tab" || !dialog) return;
    const items = [...dialog.querySelectorAll<HTMLButtonElement>("button:not([disabled])")];
    if (!items.length) return;
    const first = items[0];
    const last = items[items.length - 1];
    if (e.shiftKey && (document.activeElement === first || document.activeElement === dialog)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };

  return (
    <>
      {id && (
        <div
          className={`fixed inset-0 z-[110] flex items-center justify-center p-4 bg-stone-900/30 dark:bg-black/50 transition-opacity
                      ${closing ? "opacity-0 duration-150 ease-in" : "opacity-100 duration-200 ease-out"}`}
          onKeyDown={onKeyDown}
        >
          <div
            ref={dialogRef}
            tabIndex={-1}
            role="dialog"
            aria-modal="true"
            aria-labelledby="whats-new-title"
            aria-describedby="whats-new-steps"
            className="whats-new-dialog relative w-full max-w-[440px] max-h-full overflow-auto rounded-2xl p-3 outline-none
                       border border-stone-200/70 dark:border-white/[0.08]
                       bg-white text-stone-900 dark:bg-stone-900 dark:text-stone-50
                       shadow-[0_24px_70px_rgba(28,25,23,0.24)]"
          >
            <button
              onClick={close}
              aria-label={t("whatsNew.close")}
              className={`absolute right-5 top-5 z-10 flex h-7 w-7 items-center justify-center rounded-md
                          border border-stone-200 bg-white text-stone-400 transition-colors
                          hover:bg-stone-100 hover:text-stone-700
                          dark:border-white/[0.08] dark:bg-stone-900 dark:text-stone-500
                          dark:hover:bg-white/[0.06] dark:hover:text-stone-200 ${FOCUS_RING}`}
            >
              <X className="h-3.5 w-3.5" />
            </button>

            <div ref={stageRef} className="relative aspect-[432/270] overflow-hidden rounded-xl bg-stone-100 dark:bg-stone-800">
              <canvas
                ref={canvasRef}
                role="img"
                aria-label={t("whatsNew.demoLabel")}
                className="absolute inset-0 block h-full w-full"
              />
              <div ref={keycapRef} className="whats-new-keycap" aria-hidden="true">
                {IS_MAC ? "⌘ C" : "Ctrl C"}
              </div>
              <div ref={cheerRef} className="whats-new-cheer" aria-hidden="true">
                <b>{t("whatsNew.demoCheer")}</b>
                <span>
                  <Check className="h-[1em] w-[1em]" strokeWidth={3} />
                  {t("whatsNew.demoSaved")}
                </span>
              </div>
              {poster && (
                <button
                  onClick={() => demoRef.current?.playOnce()}
                  className={`absolute bottom-2.5 left-2.5 flex items-center gap-1 rounded-full py-1 pl-2 pr-2.5
                              border border-stone-200 bg-white text-xs font-semibold text-stone-900 shadow-sm
                              dark:border-white/[0.08] dark:bg-stone-900 dark:text-stone-50 ${FOCUS_RING}`}
                >
                  <Play className="h-3 w-3" />
                  {t("whatsNew.replay")}
                </button>
              )}
            </div>

            <div className="px-2.5 pb-1.5 pt-[18px]">
              <p className="flex items-center gap-2 text-xs font-semibold text-orange-700 dark:text-orange-300">
                {t("whatsNew.eyebrow")}
                {version && (
                  <span className="font-mono text-[11px] font-medium text-stone-400 dark:text-stone-500">v{version}</span>
                )}
              </p>
              <h2 id="whats-new-title" className="mb-3.5 mt-1.5 text-xl font-bold leading-snug tracking-tight">
                {t("whatsNew.title")}
              </h2>
              <ol id="whats-new-steps" className="mb-3.5 grid gap-2">
                {STEPS.map((n) => (
                  <li
                    key={n}
                    className={`grid grid-cols-[22px_1fr] items-center gap-2.5 text-[13px] leading-normal transition-colors
                                ${step === n ? "text-stone-900 dark:text-stone-50" : "text-stone-600 dark:text-stone-400"}`}
                  >
                    <span
                      className={`flex h-[22px] w-[22px] items-center justify-center rounded-full font-mono text-[11px] font-semibold transition-colors
                                  ${step === n
                                    ? "bg-orange-50 text-orange-700 dark:bg-orange-500/20 dark:text-orange-300"
                                    : "bg-stone-100 text-stone-400 dark:bg-white/[0.06] dark:text-stone-500"}`}
                    >
                      {n}
                    </span>
                    {t(`whatsNew.step${n}`)}
                  </li>
                ))}
              </ol>
              {autoCapture && (
                <p className="mb-3.5 grid grid-cols-[14px_1fr] gap-2 rounded-xl bg-stone-100 px-3 py-2.5 text-xs leading-relaxed text-stone-600 dark:bg-white/[0.05] dark:text-stone-300">
                  <Info className="mt-0.5 h-3.5 w-3.5 text-orange-700 dark:text-orange-300" />
                  {t("whatsNew.autoNote")}
                </p>
              )}
              <p className="mb-4 text-xs leading-relaxed text-stone-400 dark:text-stone-500">{t("whatsNew.footnote")}</p>
              <div className="flex gap-2.5">
                <button
                  onClick={close}
                  className={`h-10 flex-1 rounded-md border border-stone-200 bg-white text-sm font-medium text-stone-600
                              transition-colors hover:bg-stone-50 hover:text-stone-900
                              dark:border-white/[0.08] dark:bg-white/[0.03] dark:text-stone-300
                              dark:hover:bg-white/[0.06] dark:hover:text-stone-50 ${FOCUS_RING}`}
                >
                  {t("whatsNew.later")}
                </button>
                <button
                  onClick={enable}
                  disabled={enabled}
                  className={`flex h-10 flex-[1.4] items-center justify-center gap-1.5 rounded-md text-sm font-semibold text-white
                              transition-colors ${FOCUS_RING}
                              ${enabled ? "bg-[#16A34A]" : "bg-[#F97316] hover:bg-[#EA580C]"}`}
                >
                  {enabled && <Check className="h-4 w-4" strokeWidth={3} />}
                  {enabled ? t("whatsNew.enabled") : t("whatsNew.enable")}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {toast && (
        <div
          role="status"
          aria-live="polite"
          className="whats-new-toast fixed bottom-6 left-1/2 z-[110] flex -translate-x-1/2 items-center gap-2 rounded-xl
                     border border-transparent bg-stone-900 px-3.5 py-2.5 text-[13px] font-medium text-stone-50
                     shadow-[0_8px_24px_rgba(28,25,23,0.24)] dark:border-white/[0.08] dark:bg-stone-800"
        >
          <Check className="h-4 w-4 text-green-400" strokeWidth={3} />
          {t("whatsNew.toast")}
        </div>
      )}
    </>
  );
}
