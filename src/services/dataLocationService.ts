import { invoke } from "@tauri-apps/api/core";

export interface DataLocationInfo {
  path: string;
  is_default: boolean;
  size_mb: number;
  /** The old copy left behind by the last move, while it is still there. */
  previous: { path: string; size_mb: number } | null;
}

export interface PickedDataFolder {
  picked: string;
  /** Where the data would go: the picked folder, or an "OpenWiki" folder inside it. */
  target: string;
}

export function getDataLocation(): Promise<DataLocationInfo> {
  return invoke("get_data_location");
}

export function pickDataFolder(): Promise<PickedDataFolder | null> {
  return invoke("pick_data_folder");
}

/** Copies everything, switches over and restarts OpenWiki; only resolves/rejects on failure. */
export function moveDataDir(picked: string): Promise<void> {
  return invoke("move_data_dir", { picked });
}

export function removePreviousData(): Promise<void> {
  return invoke("remove_previous_data");
}

export function openPreviousDataFolder(): Promise<void> {
  return invoke("open_previous_data_folder");
}
