import { invoke } from "@tauri-apps/api/core";
import type { CapturedContent } from "../types/content";

interface DateEntry {
  date: string; // "YYYY-MM-DD"
  count: number;
}

export interface ImportSummary {
  imported: number;
  skipped: number;
  failed: number;
  imported_ids?: string[];
}

export interface LocalRawSyncSummary {
  root: string;
  files_found: number;
  created: number;
  updated: number;
  skipped: number;
  failed: number;
  removed: number;
  imported_ids?: string[];
  counts?: Record<string, number>;
}

export async function getDatesWithContent(): Promise<DateEntry[]> {
  const result = await invoke<[string, number][]>("get_dates_with_content");
  return result.map(([date, count]) => ({ date, count }));
}

export async function getContentForDate(
  date: string
): Promise<CapturedContent[]> {
  return invoke<CapturedContent[]>("get_content_for_date", { date });
}

export async function exportDay(date: string): Promise<string> {
  return invoke<string>("export_day_markdown", { date });
}

export async function exportAll(): Promise<number> {
  return invoke<number>("export_all_markdown");
}

export async function exportDateRange(
  start: string,
  end: string
): Promise<number> {
  return invoke<number>("export_date_range_markdown", { start, end });
}

/** Export all content into a single markdown file. Returns file path. Reveals in Finder automatically. */
export async function exportAllSingle(): Promise<string> {
  return invoke<string>("export_all_single");
}

/** Export a date range into a single markdown file. Returns file path. Reveals in Finder automatically. */
export async function exportRangeSingle(start: string, end: string): Promise<string> {
  return invoke<string>("export_range_single", { start, end });
}

export async function renderSelectedMarkdown(ids: string[]): Promise<string> {
  return invoke<string>("render_selected_markdown", { ids });
}

export async function exportSelectedSingle(ids: string[]): Promise<string> {
  return invoke<string>("export_selected_single", { ids });
}

export async function openContentFile(contentId: string): Promise<string> {
  return invoke<string>("open_content_file", { contentId });
}

export async function getExportDir(): Promise<string> {
  return invoke<string>("get_export_dir");
}

export async function setExportDir(path: string): Promise<void> {
  return invoke("set_export_dir", { path });
}

export async function openExportDir(): Promise<void> {
  return invoke("open_export_dir");
}

export async function pickImportFiles(): Promise<string[]> {
  return invoke<string[]>("pick_import_files");
}

export async function pickImportDirectory(): Promise<string> {
  return invoke<string>("pick_import_directory");
}

export async function importFiles(paths: string[]): Promise<ImportSummary> {
  return invoke<ImportSummary>("import_files", { paths });
}

export async function importDirectory(path: string): Promise<ImportSummary> {
  return invoke<ImportSummary>("import_directory", { path });
}

export async function syncLocalRawDirectory(path: string): Promise<LocalRawSyncSummary> {
  return invoke<LocalRawSyncSummary>("sync_local_raw_directory", { path });
}

export async function searchContent(query: string): Promise<CapturedContent[]> {
  return invoke<CapturedContent[]>("search_content", { query });
}
