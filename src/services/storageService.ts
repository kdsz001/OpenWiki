import { invoke } from "@tauri-apps/api/core";
import type { CapturedContent } from "../types/content";

export interface StorageInfo {
  total_items: number;
  disk_usage_mb: number;
}

export interface MarkdownImportEntry {
  file_name: string;
  content: string;
}

export interface MarkdownImportResult {
  imported: CapturedContent[];
  skipped_duplicates: number;
  skipped_invalid: number;
  failed: string[];
}

export type ContentImportKind = "markdown" | "text" | "image" | "document";

export interface ContentImportEntry {
  file_name: string;
  kind: ContentImportKind;
  text?: string;
  data_base64?: string;
}

export interface ContentImportResult {
  imported: CapturedContent[];
  skipped_duplicates: number;
  skipped_invalid: number;
  failed: string[];
}

export interface UrlImportEntry {
  url: string;
}

export interface UrlImportQueuedResult {
  job_id: string;
  total: number;
  batch_size: number;
}

export interface UrlImportProgressEvent {
  job_id: string;
  total: number;
  processed: number;
  imported: number;
  skipped_duplicates: number;
  skipped_invalid: number;
  failed: number;
  imported_ids: string[];
  first_failure: string | null;
  done: boolean;
}

export async function getAllContent(
  limit?: number,
  offset?: number
): Promise<CapturedContent[]> {
  return invoke("get_all_content", { limit, offset });
}

export async function getStorageInfo(): Promise<StorageInfo> {
  return invoke("get_storage_info");
}

export async function deleteContent(id: string): Promise<void> {
  return invoke("delete_content", { id });
}

export async function retryUrlFetch(contentId: string): Promise<void> {
  return invoke("retry_url_fetch", { contentId });
}

export async function ocrImage(contentId: string): Promise<string> {
  return invoke("ocr_image", { contentId });
}

export async function getContentsByIds(ids: string[]): Promise<CapturedContent[]> {
  return invoke("get_contents_by_ids", { ids });
}

export async function importMarkdownFiles(
  entries: MarkdownImportEntry[]
): Promise<MarkdownImportResult> {
  return invoke("import_markdown_files", { entries });
}

export async function importContentFiles(
  entries: ContentImportEntry[]
): Promise<ContentImportResult> {
  return invoke("import_content_files", { entries });
}

export async function importUrls(
  entries: UrlImportEntry[],
  jobId?: string
): Promise<UrlImportQueuedResult> {
  return invoke("import_urls", { entries, jobId });
}

export async function saveSpotlightContent(
  contentType: string,
  rawText: string | null,
  imagePath: string | null,
  sourceApp: string,
  userNote: string
): Promise<CapturedContent> {
  return invoke("save_spotlight_content", {
    contentType,
    rawText,
    imagePath,
    sourceApp,
    userNote,
  });
}
