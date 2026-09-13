import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import BubbleView from "./BubbleView";
import FootballBall from "../features/football/FootballBall";
import type { FootballLayout } from "../features/football/types";

/** The `/bubble` window: the classic circle/bar bubble, or the football style's input window. */
export default function BubbleEntry() {
  const [mode, setMode] = useState<"loading" | "classic" | FootballLayout>("loading");

  useEffect(() => {
    let cancelled = false;
    invoke<FootballLayout | null>("get_football_layout")
      .then((layout) => {
        if (!cancelled) setMode(layout ?? "classic");
      })
      .catch(() => {
        if (!cancelled) setMode("classic");
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (mode === "loading") return null;
  if (mode === "classic") return <BubbleView />;
  return <FootballBall layout={mode} />;
}
