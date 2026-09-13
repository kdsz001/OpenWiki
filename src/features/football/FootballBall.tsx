import { useCallback, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LogicalPosition, LogicalSize } from "@tauri-apps/api/dpi";
import { emitTo, listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useTranslation } from "react-i18next";
import { playSfx, unlockAudio } from "./sfx";
import {
  FIELD_LABEL,
  FOOTBALL_EVENTS,
  type FootballLayout,
  type FootballResult,
  type PointerInput,
  type PointerKind,
  type SfxCue,
} from "./types";
import "./football.css";

interface PendingCapture {
  content_type: string;
  preview: string;
  source_app: string;
  raw_text: string | null;
  image_path: string | null;
}

type Stage = "aiming" | "saving" | "saved" | "failed";

const CARD_W = 320;
const CARD_H = 140;
const FAILURE_SECONDS = 10;

function todayKey() {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** "Goal #N today": a small per-day counter kept in settings. */
async function countGoal(): Promise<number> {
  const today = todayKey();
  let count = 1;
  try {
    const settings = await invoke<Record<string, string>>("get_settings");
    const previous = parseInt(settings.football_goals_count ?? "0", 10);
    if (settings.football_goals_date === today && previous > 0) count = previous + 1;
    await invoke("update_setting", { key: "football_goals_date", value: today });
    await invoke("update_setting", { key: "football_goals_count", value: String(count) });
  } catch {
    // The counter is decoration only; the capture is already saved.
  }
  return count;
}

/** Returns null when saved, otherwise the error message. */
async function saveCapture(capture: PendingCapture): Promise<string | null> {
  try {
    await invoke("confirm_capture", {
      contentType: capture.content_type,
      preview: capture.preview,
      sourceApp: capture.source_app,
      rawText: capture.raw_text,
      imagePath: capture.image_path,
      userNote: null,
    });
    return null;
  } catch (e) {
    const message = typeof e === "string" ? e : (e as Error)?.message ?? String(e);
    // "Moved to top" is the dedup signal: the content already exists, which counts as saved.
    return message === "Moved to top" ? null : message;
  }
}

/**
 * The football bubble's input window (label "bubble"). It sits invisibly on the
 * ball, forwards clicks and drags to the field window, saves the capture as soon
 * as the ball is kicked and plays the sounds (only this window gets the user
 * gesture that audio needs). If saving fails it becomes the retry card.
 */
export default function FootballBall({ layout }: { layout: FootballLayout }) {
  const { t } = useTranslation("common");
  const winRef = useRef(getCurrentWebviewWindow());
  const pendingRef = useRef<PendingCapture | null>(null);
  const stageRef = useRef<Stage>("aiming");
  const doneRef = useRef<FootballResult | null>(null);
  const pressedRef = useRef(false);
  const [failure, setFailure] = useState<{ message: string; nonce: number } | null>(null);
  const [cardVisible, setCardVisible] = useState(false);
  const [retrying, setRetrying] = useState(false);
  const [secondsLeft, setSecondsLeft] = useState(FAILURE_SECONDS);

  const send = useCallback((event: string, payload?: unknown) => {
    void emitTo(FIELD_LABEL, event, payload);
  }, []);

  const close = useCallback(() => {
    void winRef.current.close();
  }, []);

  // dismiss_capture closes this window from the backend. Closing it again here raced with that
  // teardown, so the window closes itself only when the call failed.
  const discard = useCallback(async () => {
    try {
      await invoke("dismiss_capture", { imagePath: pendingRef.current?.image_path ?? null });
    } catch {
      close();
    }
  }, [close]);

  /** After a successful save, close through the backend (dismiss_capture), the same way the classic bubble closes. */
  const finishSaved = useCallback(async () => {
    try {
      await invoke("dismiss_capture", { imagePath: null });
    } catch {
      close(); // Already saved; closing is all that matters here.
    }
  }, [close]);

  const showFailureCard = useCallback(async () => {
    const w = layout.work;
    const x = layout.side === "left" ? w.x + 28 : w.x + w.w - 28 - CARD_W;
    const y = w.y + w.h - 28 - CARD_H;
    const win = winRef.current;
    try {
      await win.setIgnoreCursorEvents(false);
      await win.setSize(new LogicalSize(CARD_W, CARD_H));
      await win.setPosition(new LogicalPosition(layout.originX + x, layout.originY + y));
    } catch {
      // Keep the card usable even if the native resize fails.
    }
    setSecondsLeft(FAILURE_SECONDS);
    setCardVisible(true);
  }, [layout]);

  const save = useCallback(async () => {
    const capture = pendingRef.current;
    stageRef.current = "saving";
    const error = capture ? await saveCapture(capture) : null;
    if (error !== null) {
      stageRef.current = "failed";
      setFailure({ message: error, nonce: Date.now() });
      send(FOOTBALL_EVENTS.failed);
      if (doneRef.current) void showFailureCard();
      return;
    }
    stageRef.current = "saved";
    if (capture) send(FOOTBALL_EVENTS.saved, await countGoal());
    if (doneRef.current) void finishSaved();
  }, [finishSaved, send, showFailureCard]);

  const retry = useCallback(async () => {
    const capture = pendingRef.current;
    if (!capture || retrying) return;
    setRetrying(true);
    const error = await saveCapture(capture);
    setRetrying(false);
    if (error === null) {
      await countGoal();
      void finishSaved();
    } else {
      setFailure({ message: error, nonce: Date.now() });
      setSecondsLeft(FAILURE_SECONDS);
    }
  }, [finishSaved, retrying]);

  const copyError = useCallback(async () => {
    if (!failure) return;
    const report = [
      "OpenWiki 保存失败报告",
      `时间: ${new Date().toLocaleString()}`,
      `错误: ${failure.message}`,
    ].join("\n");
    try {
      await invoke("write_clipboard_text", { text: report });
    } catch (e) {
      console.error("Copy failed:", e);
    }
  }, [failure]);

  // Pending content, newer copies, and the field's messages.
  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const timer = window.setTimeout(() => {
      invoke<PendingCapture | null>("get_pending_capture")
        .then((data) => {
          if (!disposed && data) pendingRef.current = data;
        })
        .catch((e) => console.error("get_pending_capture failed:", e));
    }, 0);

    const win = winRef.current;
    Promise.all([
      listen<PendingCapture>("capture:pending", (e) => {
        // A newer copy while still aiming replaces the content and restarts the countdown.
        if (stageRef.current !== "aiming") return;
        const previous = pendingRef.current?.image_path;
        if (previous && previous !== e.payload.image_path) {
          invoke("cleanup_pending_capture", { imagePath: previous }).catch(() => {});
        }
        pendingRef.current = e.payload;
        send(FOOTBALL_EVENTS.restart);
      }),
      win.listen(FOOTBALL_EVENTS.shot, () => {
        // The ball is flying now: stop catching clicks meant for the app underneath.
        void win.setIgnoreCursorEvents(true);
        void save();
      }),
      win.listen<FootballResult>(FOOTBALL_EVENTS.done, (e) => {
        doneRef.current = e.payload;
        if (e.payload === "dismissed") void discard();
        else if (stageRef.current === "saved") void finishSaved();
        else if (stageRef.current === "failed") void showFailureCard();
        // Still saving: save() closes or shows the card when it finishes.
      }),
      win.listen<SfxCue>(FOOTBALL_EVENTS.sfx, (e) => playSfx(e.payload.name, e.payload.value)),
    ])
      .then((list) => {
        if (disposed) list.forEach((unlisten) => unlisten());
        else unlisteners.push(...list);
      })
      .catch((e) => console.error("Football listeners failed:", e));

    return () => {
      disposed = true;
      window.clearTimeout(timer);
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [discard, finishSaved, save, send, showFailureCard]);

  // Enter shoots, Esc cancels the aim or dismisses (when this window has focus).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (stageRef.current !== "aiming" || (e.key !== "Enter" && e.key !== "Escape")) return;
      e.preventDefault();
      unlockAudio();
      send(FOOTBALL_EVENTS.key, e.key);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [send]);

  // The failure card closes itself after a while if the user walked away.
  const failureNonce = failure?.nonce ?? 0;
  useEffect(() => {
    if (!cardVisible) return;
    let remaining = FAILURE_SECONDS;
    const timer = window.setInterval(() => {
      remaining -= 1;
      setSecondsLeft(remaining);
      if (remaining <= 0) {
        window.clearInterval(timer);
        void discard();
      }
    }, 1000);
    return () => window.clearInterval(timer);
  }, [cardVisible, failureNonce, discard]);

  const forward = (type: PointerKind, e: ReactPointerEvent<HTMLDivElement>) => {
    send(FOOTBALL_EVENTS.input, { type, x: e.clientX, y: e.clientY, buttons: e.buttons } satisfies PointerInput);
  };

  const release = (type: "up" | "cancel", e: ReactPointerEvent<HTMLDivElement>) => {
    if (!pressedRef.current) return;
    pressedRef.current = false;
    forward(type, e);
  };

  if (cardVisible && failure) {
    return (
      <div className="select-none" style={{ width: CARD_W, height: CARD_H, background: "transparent" }}>
        <div
          style={{
            width: CARD_W,
            borderRadius: 14,
            background: "rgb(15, 15, 30)",
            boxShadow: [
              "0 8px 32px rgba(0, 0, 0, 0.5)",
              "0 0 12px rgba(220, 38, 38, 0.18)",
              "inset 0 1px 0 rgba(255, 255, 255, 0.08)",
              "inset 0 0 0 1px rgba(220, 38, 38, 0.22)",
            ].join(", "),
            padding: "12px 14px",
            display: "flex",
            flexDirection: "column",
            gap: 10,
            position: "relative",
            overflow: "hidden",
          }}
        >
          <div style={{ display: "flex", alignItems: "flex-start", gap: 10 }}>
            <div
              style={{
                width: 28,
                height: 28,
                borderRadius: "50%",
                background: "rgba(220, 38, 38, 0.15)",
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                flexShrink: 0,
              }}
            >
              <svg width="14" height="14" fill="none" viewBox="0 0 24 24" stroke="#F87171" strokeWidth={2.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z" />
              </svg>
            </div>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 2 }}>
                <span style={{ fontSize: 13, fontWeight: 600, color: "#F87171" }}>{t("bubble.saveFailed")}</span>
                <span style={{ fontSize: 10, fontFamily: "JetBrains Mono, monospace", color: "rgba(255, 255, 255, 0.25)" }}>
                  {secondsLeft}s
                </span>
              </div>
              <div
                style={{
                  fontSize: 12,
                  color: "rgba(255, 255, 255, 0.65)",
                  lineHeight: 1.5,
                  wordBreak: "break-word",
                  maxHeight: 48,
                  overflowY: "auto",
                }}
              >
                {failure.message}
              </div>
            </div>
          </div>
          <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
            <button
              onClick={() => void retry()}
              disabled={retrying}
              style={{
                height: 28,
                padding: "0 10px",
                borderRadius: 8,
                background: "rgba(249, 115, 22, 0.2)",
                border: "1px solid rgba(249, 115, 22, 0.3)",
                color: "#FDBA74",
                fontSize: 12,
                fontWeight: 500,
                cursor: "pointer",
                whiteSpace: "nowrap",
              }}
            >
              {retrying ? t("bubble.saving") : t("bubble.retry")}
            </button>
            <button
              onClick={() => void copyError()}
              style={{
                height: 28,
                padding: "0 10px",
                borderRadius: 8,
                background: "rgba(255, 255, 255, 0.05)",
                border: "1px solid rgba(255, 255, 255, 0.08)",
                color: "rgba(255, 255, 255, 0.6)",
                fontSize: 12,
                fontWeight: 500,
                cursor: "pointer",
                whiteSpace: "nowrap",
              }}
            >
              {t("bubble.copyError")}
            </button>
            <button
              onClick={() => void discard()}
              aria-label={t("bubble.dismiss")}
              style={{
                width: 28,
                height: 28,
                borderRadius: 8,
                background: "rgba(255, 255, 255, 0.05)",
                border: "1px solid rgba(255, 255, 255, 0.08)",
                color: "rgba(255, 255, 255, 0.55)",
                cursor: "pointer",
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                marginLeft: "auto",
              }}
            >
              <svg width="13" height="13" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          </div>
          <div style={{ position: "absolute", left: 0, right: 0, bottom: 0, height: 2, background: "rgba(255, 255, 255, 0.03)" }}>
            <div
              style={{
                height: "100%",
                width: `${(secondsLeft / FAILURE_SECONDS) * 100}%`,
                background: "linear-gradient(90deg, #DC2626, #F87171)",
                transition: "width 1s linear",
              }}
            />
          </div>
        </div>
      </div>
    );
  }

  return (
    <div
      className="football-hit"
      onPointerDown={(e) => {
        if (e.button !== 0 || stageRef.current !== "aiming") return;
        unlockAudio();
        e.currentTarget.setPointerCapture(e.pointerId);
        pressedRef.current = true;
        forward("down", e);
      }}
      onPointerMove={(e) => {
        if (pressedRef.current) forward("move", e);
      }}
      onPointerUp={(e) => release("up", e)}
      onPointerCancel={(e) => release("cancel", e)}
      onLostPointerCapture={(e) => release("up", e)}
      onPointerEnter={(e) => forward("enter", e)}
      onPointerLeave={(e) => {
        if (!pressedRef.current) forward("leave", e);
      }}
    />
  );
}
