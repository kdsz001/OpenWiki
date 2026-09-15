import { create } from "zustand";
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { type UpdateInfo } from "../services/updateService";

export type UpdatePhase = "idle" | "downloading" | "ready" | "installing" | "failed";

interface UpdateState {
  /** The newer release the GitHub check found (background or manual). */
  info: UpdateInfo | null;
  phase: UpdatePhase;
  /** Share of the package downloaded (0–1), or null while its size is unknown. */
  progress: number | null;
  failure: "download" | "install" | null;
  error: string;
  /** Settings → App updates is open and shows this state itself, so the corner dialog stays hidden. */
  inlineVisible: boolean;
  /** The corner dialog was put off ("Later"); the package stays ready to install from Settings. */
  dialogDismissed: boolean;
  /** Downloads this release in the background; does nothing while the same version is under way. */
  prepare: (info: UpdateInfo) => void;
  install: () => Promise<void>;
  retry: () => void;
  dismissDialog: () => void;
  setInlineVisible: (visible: boolean) => void;
}

// The downloaded package is a native resource rather than state, so it lives outside the store.
let downloaded: Update | null = null;
// Every download attempt gets a number, so a superseded one cannot overwrite newer state.
let attempt = 0;

const messageOf = (err: unknown) => (err instanceof Error ? err.message : String(err));

function release(update: Update | null) {
  update?.close().catch((err) => console.error("[update] failed to release the update:", err));
}

/**
 * Preparing an app update, shared by the corner dialog (UpdateBanner) and Settings → App updates.
 * A newer release found by the GitHub check downloads in the background right away; once it is
 * ready, either place installs it and relaunches.
 */
export const useUpdateStore = create<UpdateState>((set, get) => ({
  info: null,
  phase: "idle",
  progress: null,
  failure: null,
  error: "",
  inlineVisible: false,
  dialogDismissed: false,

  prepare: (info) => {
    const { info: current, phase } = get();
    const underWay = phase === "downloading" || phase === "ready" || phase === "installing";
    if (underWay && current?.version === info.version) return;

    const run = ++attempt;
    release(downloaded);
    downloaded = null;
    set({ info, phase: "downloading", progress: null, failure: null, error: "", dialogDismissed: false });

    const download = async () => {
      const update = await check();
      if (!update) throw new Error("Update package is not available");
      if (run !== attempt) {
        release(update);
        return;
      }
      let total = 0;
      let received = 0;
      let shown = -1;
      const onEvent = (event: DownloadEvent) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? 0;
        } else if (event.event === "Progress" && total > 0) {
          received += event.data.chunkLength;
          const percent = Math.min(100, Math.floor((received / total) * 100));
          if (percent !== shown && run === attempt) {
            shown = percent;
            set({ progress: percent / 100 });
          }
        }
      };
      try {
        await update.download(onEvent);
      } catch (err) {
        release(update);
        throw err;
      }
      if (run !== attempt) {
        release(update);
        return;
      }
      downloaded = update;
      set({ phase: "ready", progress: 1 });
    };

    download().catch((err) => {
      console.error("[update] background download failed:", err);
      if (run === attempt) set({ phase: "failed", failure: "download", error: messageOf(err) });
    });
  },

  install: async () => {
    const update = downloaded;
    if (get().phase !== "ready" || !update) return;
    set({ phase: "installing", failure: null, error: "" });
    try {
      await update.install();
      await relaunch();
    } catch (err) {
      console.error("[update] install failed:", err);
      set({ phase: "failed", failure: "install", error: messageOf(err) });
    }
  },

  retry: () => {
    const { info } = get();
    if (!info) return;
    set({ phase: "idle" });
    get().prepare(info);
  },

  dismissDialog: () => {
    if (get().phase !== "installing") set({ dialogDismissed: true });
  },

  setInlineVisible: (visible) => set({ inlineVisible: visible }),
}));
