import { invoke } from "@tauri-apps/api/core";

export interface WorkspacePaths {
  initialized: boolean;
  root: string;
  inbox: string;
  raw: string;
  wiki_root: string;
  wiki_cases: string;
  wiki_concepts: string;
  wiki_themes: string;
  wiki_dashboards: string;
  wiki_drafts: string;
  wiki_candidates: string;
  insights: string;
}

export async function getWorkspacePaths(): Promise<WorkspacePaths> {
  return invoke<WorkspacePaths>("get_workspace_paths");
}

export async function initializeWorkspaceRoot(path?: string): Promise<WorkspacePaths> {
  return invoke<WorkspacePaths>("initialize_workspace_root", {
    path: path?.trim() ? path.trim() : null,
  });
}

export async function openWorkspaceRoot(): Promise<string> {
  return invoke<string>("open_workspace_root");
}
