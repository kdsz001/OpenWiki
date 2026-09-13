import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emitTo } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { Check } from "lucide-react";
import { useTranslation } from "react-i18next";
import { FootballEngine, type Point } from "./engine";
import {
  BALL_HIT_SIZE,
  BALL_LABEL,
  FOOTBALL_EVENTS,
  type FootballLayout,
  type FootballResult,
  type PointerInput,
  type SfxCue,
} from "./types";
import "./football.css";

type SaveState = { kind: "pending" } | { kind: "saved"; count: number } | { kind: "failed" };

interface Pop {
  cheer: string;
  x: number;
  y: number;
}

/**
 * Full-screen, click-through window (label "football-field") that draws the ball,
 * the goal and every animation. It never receives mouse input itself: the small
 * ball window forwards clicks and drags here.
 */
export default function FootballField() {
  const { t } = useTranslation("common");
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [tip, setTip] = useState<Point | null>(null);
  const [pop, setPop] = useState<Pop | null>(null);
  const [popLeaving, setPopLeaving] = useState(false);
  const [save, setSave] = useState<SaveState>({ kind: "pending" });

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const win = getCurrentWebviewWindow();
    let engine: FootballEngine | null = null;
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    const init = async () => {
      const [layout, settings] = await Promise.all([
        invoke<FootballLayout | null>("get_football_layout"),
        invoke<Record<string, string>>("get_settings").catch((): Record<string, string> => ({})),
      ]);
      if (disposed) return;
      if (!layout) {
        void win.close();
        return;
      }
      const seconds = parseInt(settings.countdown_seconds ?? "", 10);
      const sound = settings.football_sound !== "false";
      const game = new FootballEngine(canvas, layout, {
        countdown: seconds >= 1 && seconds <= 30 ? seconds : 5,
        defaultAction: settings.default_action === "save" ? "save" : "dismiss",
        learned: settings.football_tip_seen === "true",
        texts: {
          cheers: [t("bubble.football.nice"), t("bubble.football.beauty"), t("bubble.football.goal")],
          worldie: t("bubble.football.worldie"),
          banana: t("bubble.football.banana"),
          rocket: t("bubble.football.rocket"),
          tapIn: t("bubble.football.tapIn"),
          rebound: t("bubble.football.rebound"),
          doubleRebound: t("bubble.football.doubleRebound"),
          winner: t("bubble.football.winner"),
        },
        onBallReady: (x, y) => {
          invoke("show_football_ball", { x, y, size: BALL_HIT_SIZE }).catch((e) =>
            console.error("show_football_ball failed:", e),
          );
        },
        onShot: () => void emitTo(BALL_LABEL, FOOTBALL_EVENTS.shot),
        onDone: (result: FootballResult) => {
          // Tell the ball window how it ended, then remove the field.
          void emitTo(BALL_LABEL, FOOTBALL_EVENTS.done, result).finally(() => void win.close());
        },
        onSfx: (name, value) => {
          if (sound) void emitTo(BALL_LABEL, FOOTBALL_EVENTS.sfx, { name, value } satisfies SfxCue);
        },
        onLearned: () => {
          invoke("update_setting", { key: "football_tip_seen", value: "true" }).catch(() => {});
        },
        onTip: setTip,
        onPop: (cheer, x, y) => {
          setPopLeaving(false);
          setPop({ cheer, x, y });
        },
        onPopHide: () => setPopLeaving(true),
      });
      const listeners = await Promise.all([
        win.listen<PointerInput>(FOOTBALL_EVENTS.input, (e) => game.input(e.payload)),
        win.listen<string>(FOOTBALL_EVENTS.key, (e) => game.key(e.payload)),
        win.listen(FOOTBALL_EVENTS.restart, () => game.restartCountdown()),
        win.listen<number>(FOOTBALL_EVENTS.saved, (e) => {
          setSave({ kind: "saved", count: e.payload });
          game.setSaveResult();
        }),
        win.listen(FOOTBALL_EVENTS.failed, () => {
          setSave({ kind: "failed" });
          game.setSaveResult();
        }),
      ]);
      if (disposed) {
        listeners.forEach((unlisten) => unlisten());
        return;
      }
      unlisteners.push(...listeners);
      engine = game;
      game.start();
    };

    init().catch((e) => {
      console.error("Football field failed to start:", e);
      void win.close();
    });

    return () => {
      disposed = true;
      engine?.destroy();
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [t]);

  return (
    <div className="football-field">
      <canvas ref={canvasRef} className="football-canvas" />
      {tip && (
        <div className="football-tip" style={{ left: tip.x, top: tip.y }}>
          {t("bubble.football.tip")}
        </div>
      )}
      {pop && (
        <div className="football-pop-anchor" style={{ left: pop.x, top: pop.y }}>
          <div className={`football-pop ${popLeaving ? "is-leaving" : "is-showing"}`}>
            <b>{pop.cheer}</b>
            {save.kind === "saved" && (
              <span className="football-pill is-saved">
                <Check size={12} strokeWidth={3} />
                {t("bubble.football.saved", { count: save.count })}
              </span>
            )}
            {save.kind === "failed" && (
              <span className="football-pill is-failed">{t("bubble.saveFailed")}</span>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
