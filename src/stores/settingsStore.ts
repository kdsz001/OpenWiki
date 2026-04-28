import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { getSettings, updateSetting, checkXReaderStatus, type XReaderStatus } from "../services/settingsService";
import { setAppLanguage, getSystemLanguage, initLanguageFromSettings } from "../i18n";

export type AIProvider = "anthropic" | "openai" | "openrouter" | "dashscope" | "google" | "minimax" | "deepseek" | "ollama" | "lmstudio" | "custom";

export interface AIModelOption {
  id: string;
  label: string;
  free?: boolean;
  group?: string;
}

export const MODELS_BY_PROVIDER: Record<AIProvider, AIModelOption[]> = {
  anthropic: [
    { id: "claude-opus-4-7", label: "Claude Opus 4.7" },
    { id: "claude-sonnet-4-6", label: "Claude Sonnet 4.6" },
    { id: "claude-haiku-4-5-20251001", label: "Claude Haiku 4.5" },
  ],
  openai: [
    { id: "auto", label: "Auto" },
    { id: "gpt-5.5", label: "GPT-5.5" },
    { id: "gpt-5.5-pro", label: "GPT-5.5 Pro" },
    { id: "gpt-5.4", label: "GPT-5.4" },
    { id: "gpt-5.4-mini", label: "GPT-5.4 Mini" },
    { id: "gpt-5.4-nano", label: "GPT-5.4 Nano" },
    { id: "gpt-5.3-codex", label: "GPT-5.3 Codex" },
    { id: "gpt-5.2", label: "GPT-5.2" },
    { id: "gpt-5.2-codex", label: "GPT-5.2 Codex" },
    { id: "gpt-5.1-codex-max", label: "GPT-5.1 Codex Max" },
    { id: "gpt-5.1-codex", label: "GPT-5.1 Codex" },
    { id: "gpt-5.1-codex-mini", label: "GPT-5.1 Codex Mini" },
  ],
  openrouter: [
    // ── Auto (default) ──
    { id: "openrouter/free", label: "Auto Free", free: true, group: "Free" },
    // ── Free models ──
    { id: "nousresearch/hermes-3-llama-3.1-405b:free", label: "Hermes 3 405B", free: true, group: "Free" },
    { id: "qwen/qwen3-coder:free", label: "Qwen3 Coder 480B", free: true, group: "Free" },
    { id: "openai/gpt-oss-120b:free", label: "GPT-OSS 120B", free: true, group: "Free" },
    { id: "nvidia/nemotron-3-super-120b-a12b:free", label: "Nemotron 3 Super 120B", free: true, group: "Free" },
    { id: "qwen/qwen3-next-80b-a3b-instruct:free", label: "Qwen3 Next 80B", free: true, group: "Free" },
    { id: "meta-llama/llama-3.3-70b-instruct:free", label: "Llama 3.3 70B", free: true, group: "Free" },
    { id: "minimax/minimax-m2.5:free", label: "MiniMax M2.5", free: true, group: "Free" },
    { id: "z-ai/glm-4.5-air:free", label: "GLM 4.5 Air", free: true, group: "Free" },
    // ── More free ──
    { id: "google/gemma-4-31b-it:free", label: "Gemma 4 31B", free: true, group: "More Free" },
    { id: "google/gemma-4-26b-a4b-it:free", label: "Gemma 4 26B", free: true, group: "More Free" },
    { id: "google/gemma-3-27b-it:free", label: "Gemma 3 27B", free: true, group: "More Free" },
    { id: "nvidia/nemotron-3-nano-30b-a3b:free", label: "Nemotron 3 Nano 30B", free: true, group: "More Free" },
    { id: "openai/gpt-oss-20b:free", label: "GPT-OSS 20B", free: true, group: "More Free" },
    { id: "arcee-ai/trinity-large-preview:free", label: "Trinity Large 400B", free: true, group: "More Free" },
    // ── Anthropic ──
    { id: "anthropic/claude-opus-4.6", label: "Claude Opus 4.6", group: "Anthropic" },
    { id: "anthropic/claude-sonnet-4.6", label: "Claude Sonnet 4.6", group: "Anthropic" },
    { id: "anthropic/claude-haiku-4.5", label: "Claude Haiku 4.5", group: "Anthropic" },
    // ── OpenAI ──
    { id: "openai/gpt-5.5", label: "GPT-5.5", group: "OpenAI" },
    { id: "openai/gpt-5.5-pro", label: "GPT-5.5 Pro", group: "OpenAI" },
    { id: "openai/gpt-5.4", label: "GPT-5.4", group: "OpenAI" },
    { id: "openai/gpt-5.2", label: "GPT-5.2", group: "OpenAI" },
    { id: "openai/gpt-5.1", label: "GPT-5.1", group: "OpenAI" },
    // ── Google ──
    { id: "google/gemini-3.1-pro-preview", label: "Gemini 3.1 Pro", group: "Google" },
    { id: "google/gemini-3-pro-preview", label: "Gemini 3 Pro", group: "Google" },
    { id: "google/gemini-3-flash-preview", label: "Gemini 3 Flash", group: "Google" },
    // ── DeepSeek ──
    { id: "deepseek/deepseek-v4-pro", label: "DeepSeek V4 Pro", group: "DeepSeek" },
    { id: "deepseek/deepseek-v4-flash", label: "DeepSeek V4 Flash", group: "DeepSeek" },
    { id: "deepseek/deepseek-v3.2", label: "DeepSeek V3.2", group: "DeepSeek" },
    { id: "deepseek/deepseek-v3.2-speciale", label: "DeepSeek V3.2 Speciale", group: "DeepSeek" },
    { id: "deepseek/deepseek-r1", label: "DeepSeek R1", group: "DeepSeek" },
    // ── xAI ──
    { id: "x-ai/grok-4.20", label: "Grok 4.20", group: "xAI" },
    { id: "x-ai/grok-4.1-fast", label: "Grok 4.1 Fast", group: "xAI" },
    // ── Zhipu ──
    { id: "z-ai/glm-5.1", label: "GLM 5.1", group: "Zhipu" },
    { id: "z-ai/glm-5", label: "GLM 5", group: "Zhipu" },
    // ── Qwen ──
    { id: "qwen/qwen3.6-plus", label: "Qwen3.6 Plus", group: "Qwen" },
    { id: "qwen/qwen3-coder-next", label: "Qwen3 Coder Next", group: "Qwen" },
    // ── Meta ──
    { id: "meta-llama/llama-4-maverick", label: "Llama 4 Maverick", group: "Meta" },
    // ── Mistral ──
    { id: "mistralai/mistral-large-2512", label: "Mistral Large 3", group: "Mistral" },
  ],
  dashscope: [
    { id: "qwen3.6-plus", label: "Qwen3.6 Plus" },
    { id: "qwen-plus", label: "Qwen Plus" },
    { id: "qwen-turbo", label: "Qwen Turbo" },
    { id: "qwen-max", label: "Qwen Max" },
    { id: "qwen-long", label: "Qwen Long" },
    { id: "deepseek-v4-flash", label: "DeepSeek V4 Flash（便宜快）" },
    { id: "deepseek-v4-pro", label: "DeepSeek V4 Pro（深度思考）" },
  ],
  google: [
    { id: "auto", label: "Auto" },
    { id: "gemini-3-flash", label: "Gemini 3 Flash" },
    { id: "gemini-3-pro-low", label: "Gemini 3 Pro" },
    { id: "gemini-3-pro-high", label: "Gemini 3 Pro (Deep)" },
    { id: "gemini-3.1-pro-low", label: "Gemini 3.1 Pro" },
    { id: "gemini-3.1-pro-high", label: "Gemini 3.1 Pro (Deep)" },
    { id: "claude-sonnet-4-6", label: "Claude Sonnet 4.6" },
    { id: "claude-opus-4-6-thinking", label: "Claude Opus 4.6" },
  ],
  minimax: [
    { id: "MiniMax-M2.7", label: "MiniMax M2.7" },
    { id: "MiniMax-M2.5", label: "MiniMax M2.5" },
    { id: "MiniMax-M2.1", label: "MiniMax M2.1" },
  ],
  deepseek: [
    { id: "deepseek-v4-flash", label: "DeepSeek V4 Flash（便宜快）" },
    { id: "deepseek-v4-pro", label: "DeepSeek V4 Pro（旗舰）" },
  ],
  ollama: [
    { id: "llama3.1", label: "Llama 3.1" },
    { id: "qwen2.5", label: "Qwen 2.5" },
    { id: "gemma2", label: "Gemma 2" },
    { id: "mistral", label: "Mistral" },
  ],
  lmstudio: [],
  custom: [],
};

export const PROVIDER_LABELS: Record<AIProvider, string> = {
  anthropic: "Anthropic",
  openai: "OpenAI",
  openrouter: "OpenRouter",
  dashscope: "Qwen",
  google: "Google",
  minimax: "MiniMax",
  deepseek: "DeepSeek",
  ollama: "Ollama（本地）",
  lmstudio: "LM Studio（本地）",
  custom: "自定义",
};

export const DEFAULT_BASE_URLS: Partial<Record<AIProvider, string>> = {
  ollama: "http://localhost:11434/v1",
  lmstudio: "http://localhost:1234/v1",
  custom: "",
};

const VALID_PROVIDERS: AIProvider[] = ["anthropic", "openai", "openrouter", "dashscope", "google", "minimax", "deepseek", "ollama", "lmstudio", "custom"];

export type CaptureMode = "auto" | "confirm";
export type BubbleStyle = "circle" | "bar";
export type BubblePosition = "bottom-right" | "bottom-center" | "bottom-left" | "top-right" | "top-center" | "top-left";
export type DefaultAction = "save" | "dismiss";
export type ThemeMode = "light" | "dark" | "system";
export type LanguageMode = "system" | "zh-CN" | "en-US";

const VALID_BUBBLE_POSITIONS: BubblePosition[] = [
  "bottom-right", "bottom-center", "bottom-left",
  "top-right", "top-center", "top-left",
];

// Track the current system theme listener so we can clean it up when theme changes
let systemThemeCleanup: (() => void) | null = null;

function applyTheme(theme: ThemeMode) {
  // Clean up previous system theme listener
  if (systemThemeCleanup) {
    systemThemeCleanup();
    systemThemeCleanup = null;
  }

  const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
  const isDark =
    theme === "dark" ||
    (theme === "system" && mediaQuery.matches);
  document.documentElement.classList.toggle("dark", isDark);

  // If "system" mode, listen for OS theme changes and auto-update
  if (theme === "system") {
    const handler = (e: MediaQueryListEvent) => {
      document.documentElement.classList.toggle("dark", e.matches);
    };
    mediaQuery.addEventListener("change", handler);
    systemThemeCleanup = () => mediaQuery.removeEventListener("change", handler);
  }
}

// Patterns for detecting sensitive data (passwords, private keys, API keys, tokens, secrets)
export const SENSITIVE_PATTERNS: RegExp[] = [
  // API Keys & tokens (generic)
  /(?:api[_-]?key|apikey|access[_-]?token|auth[_-]?token|bearer)\s*[:=]\s*['"]?[A-Za-z0-9_\-./+]{16,}/i,
  // AWS keys
  /AKIA[0-9A-Z]{16}/,
  // GitHub tokens
  /gh[ps]_[A-Za-z0-9_]{36,}/,
  // Slack tokens
  /xox[bpras]-[A-Za-z0-9-]{10,}/,
  // Private keys (PEM)
  /-----BEGIN\s+(RSA\s+)?PRIVATE\s+KEY-----/,
  // SSH private keys
  /-----BEGIN\s+OPENSSH\s+PRIVATE\s+KEY-----/,
  // Password patterns
  /(?:password|passwd|pwd)\s*[:=]\s*['"]?.{4,}/i,
  // Secret patterns
  /(?:secret|client[_-]?secret)\s*[:=]\s*['"]?[A-Za-z0-9_\-./+]{8,}/i,
  // JWT tokens
  /eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}/,
  // OpenAI keys
  /sk-[A-Za-z0-9]{20,}/,
  // Anthropic keys
  /sk-ant-[A-Za-z0-9_-]{20,}/,
];

export function containsSensitiveData(text: string): boolean {
  return SENSITIVE_PATTERNS.some((pattern) => pattern.test(text));
}

interface SettingsState {
  apiKey: string;
  provider: AIProvider;
  model: string;
  customBaseUrl: string;
  theme: ThemeMode;
  languageMode: LanguageMode;
  resolvedLanguage: string;
  captureEnabled: boolean;
  captureMode: CaptureMode;
  bubbleStyle: BubbleStyle;
  bubblePosition: BubblePosition;
  defaultAction: DefaultAction;
  sensitiveFilterEnabled: boolean;
  urlReadingEnabled: boolean;
  radarIntervalDays: number;
  countdownDuration: number;
  screenshotDir: string;
  totalItems: number;
  diskUsageMB: number;
  isLoaded: boolean;
  xreaderStatus: XReaderStatus | null;

  // OpenAI OAuth
  oauthLoggedIn: boolean;
  oauthEmail: string;
  oauthLoading: boolean;

  // Gemini OAuth
  geminiOauthLoggedIn: boolean;
  geminiOauthEmail: string;
  geminiOauthLoading: boolean;
  loadGeminiOAuthStatus: () => Promise<void>;
  startGeminiOAuthLogin: () => Promise<void>;
  logoutGeminiOAuth: () => Promise<void>;

  loadFromDB: () => Promise<void>;
  setApiKey: (key: string) => void;
  setProvider: (provider: AIProvider) => void;
  setModel: (model: string) => void;
  setCustomBaseUrl: (url: string) => void;
  setTheme: (theme: ThemeMode) => void;
  setLanguageMode: (mode: LanguageMode) => void;
  setCaptureEnabled: (enabled: boolean) => void;
  setCaptureMode: (mode: CaptureMode) => void;
  setBubbleStyle: (style: BubbleStyle) => void;
  setBubblePosition: (position: BubblePosition) => void;
  setDefaultAction: (action: DefaultAction) => void;
  setSensitiveFilterEnabled: (enabled: boolean) => void;
  setUrlReadingEnabled: (enabled: boolean) => void;
  setRadarIntervalDays: (days: number) => void;
  setCountdownDuration: (seconds: number) => void;
  setScreenshotDir: (dir: string) => void;
  setStorageInfo: (totalItems: number, diskUsageMB: number) => void;
  loadXReaderStatus: () => Promise<void>;
  loadOAuthStatus: () => Promise<void>;
  startOAuthLogin: () => Promise<void>;
  logoutOAuth: () => Promise<void>;
}

export const useSettingsStore = create<SettingsState>((set) => ({
  apiKey: "",
  provider: "anthropic",
  model: "claude-sonnet-4-6",
  customBaseUrl: "",
  theme: "system",
  languageMode: "system" as LanguageMode,
  resolvedLanguage: getSystemLanguage(),
  captureEnabled: true,
  captureMode: "confirm" as CaptureMode,
  bubbleStyle: "circle" as BubbleStyle,
  bubblePosition: "bottom-right" as BubblePosition,
  defaultAction: "dismiss" as DefaultAction,
  sensitiveFilterEnabled: false,
  urlReadingEnabled: true,
  radarIntervalDays: 3,
  countdownDuration: 5,
  screenshotDir: "~/Library/Application Support/com.openwiki.app/screenshots",
  totalItems: 0,
  diskUsageMB: 0,
  isLoaded: false,
  xreaderStatus: null,

  oauthLoggedIn: false,
  oauthEmail: "",
  oauthLoading: false,

  geminiOauthLoggedIn: false,
  geminiOauthEmail: "",
  geminiOauthLoading: false,

  loadFromDB: async () => {
    try {
      const settings = await getSettings();

      const provider = VALID_PROVIDERS.includes(settings.ai_provider as AIProvider)
        ? (settings.ai_provider as AIProvider)
        : "anthropic";

      const defaultModel = MODELS_BY_PROVIDER[provider][0]?.id || "";
      const model = settings.ai_model || defaultModel;
      const customBaseUrl = settings.ai_custom_base_url || DEFAULT_BASE_URLS[provider] || "";

      // Load per-provider API key
      const providerKey = settings[`ai_api_key_${provider}` as keyof typeof settings] || "";

      // One-time migration: if legacy ai_api_key exists but NO provider-specific keys exist yet,
      // migrate it to the current active provider only
      let apiKey = providerKey;
      if (!providerKey && settings.ai_api_key) {
        const anyProviderKeyExists = VALID_PROVIDERS.some(
          (p) => !!settings[`ai_api_key_${p}` as keyof typeof settings]
        );
        if (!anyProviderKeyExists) {
          // First time: migrate legacy key to this provider
          apiKey = settings.ai_api_key;
          updateSetting(`ai_api_key_${provider}`, settings.ai_api_key).catch(() => {});
        }
      }

      const theme = (["light", "dark", "system"].includes(settings.theme)
        ? settings.theme
        : "system") as ThemeMode;

      applyTheme(theme);

      const languageMode = (["system", "zh-CN", "en-US"].includes(settings.language_mode)
        ? settings.language_mode
        : "system") as LanguageMode;
      const resolvedLanguage = languageMode === "system" ? getSystemLanguage() : languageMode;
      initLanguageFromSettings(languageMode);

      set({
        apiKey,
        provider,
        model,
        customBaseUrl,
        theme,
        languageMode,
        resolvedLanguage,
        captureEnabled: settings.capture_enabled !== "false",
        captureMode: (settings.capture_mode === "auto" ? "auto" : "confirm") as CaptureMode,
        bubbleStyle: (settings.bubble_style === "bar" ? "bar" : "circle") as BubbleStyle,
        bubblePosition: (VALID_BUBBLE_POSITIONS.includes(settings.bubble_position as BubblePosition)
          ? settings.bubble_position
          : "bottom-right") as BubblePosition,
        defaultAction: (settings.default_action === "save" ? "save" : "dismiss") as DefaultAction,
        sensitiveFilterEnabled: settings.sensitive_filter_enabled === "true",
        urlReadingEnabled: settings.url_reading_enabled !== "false",
        radarIntervalDays: parseInt(settings.radar_interval_days || "3", 10),
        countdownDuration: parseInt(settings.countdown_seconds || "5", 10),
        screenshotDir:
          settings.screenshot_dir ||
          "~/Library/Application Support/com.openwiki.app/screenshots",
        isLoaded: true,
      });

      // Load OAuth status
      try {
        const oauthStatus = await invoke<{ logged_in: boolean; email?: string }>("get_openai_oauth_status");
        set((prev) => ({ ...prev, oauthLoggedIn: oauthStatus.logged_in, oauthEmail: oauthStatus.email || "" }));
      } catch {}

      // Load Gemini OAuth status
      try {
        const geminiStatus = await invoke<{ logged_in: boolean; email?: string }>("get_gemini_oauth_status");
        set((prev) => ({ ...prev, geminiOauthLoggedIn: geminiStatus.logged_in, geminiOauthEmail: geminiStatus.email || "" }));
      } catch {}
    } catch (e) {
      console.error("Failed to load settings from DB:", e);
      applyTheme("system");
      set({ isLoaded: true });
    }
  },

  setApiKey: (key) => {
    const { provider } = useSettingsStore.getState();
    set({ apiKey: key });
    // Save to provider-specific key
    updateSetting(`ai_api_key_${provider}`, key).catch((e) =>
      console.error("Failed to save api key:", e)
    );
  },

  setProvider: async (provider) => {
    const firstModel = MODELS_BY_PROVIDER[provider][0]?.id || "";
    const defaultBaseUrl = DEFAULT_BASE_URLS[provider] || "";
    // Load the API key for the new provider
    try {
      const settings = await getSettings();
      const providerKey = settings[`ai_api_key_${provider}` as keyof typeof settings] || "";
      const savedBaseUrl = settings.ai_custom_base_url || defaultBaseUrl;
      set({ provider, model: firstModel, apiKey: providerKey, customBaseUrl: savedBaseUrl });
    } catch {
      set({ provider, model: firstModel, apiKey: "", customBaseUrl: defaultBaseUrl });
    }
    updateSetting("ai_provider", provider).catch((e) =>
      console.error("Failed to save provider:", e)
    );
    updateSetting("ai_model", firstModel).catch((e) =>
      console.error("Failed to save model:", e)
    );
  },

  setModel: (model) => {
    set({ model });
    updateSetting("ai_model", model).catch((e) =>
      console.error("Failed to save model:", e)
    );
  },

  setCustomBaseUrl: (url) => {
    set({ customBaseUrl: url });
    updateSetting("ai_custom_base_url", url).catch((e) =>
      console.error("Failed to save custom base url:", e)
    );
  },

  setTheme: (theme) => {
    set({ theme });
    applyTheme(theme);
    updateSetting("theme", theme).catch((e) =>
      console.error("Failed to save theme:", e)
    );
  },

  setLanguageMode: (mode) => {
    const resolved = mode === "system" ? getSystemLanguage() : mode;
    set({ languageMode: mode, resolvedLanguage: resolved });
    setAppLanguage(mode);
    updateSetting("language_mode", mode).catch((e) =>
      console.error("Failed to save language_mode:", e)
    );
  },

  setCaptureEnabled: (enabled) => {
    set({ captureEnabled: enabled });
    updateSetting("capture_enabled", String(enabled)).catch((e) =>
      console.error("Failed to save capture_enabled:", e)
    );
  },

  setCaptureMode: (mode) => {
    set({ captureMode: mode });
    updateSetting("capture_mode", mode).catch((e) =>
      console.error("Failed to save capture_mode:", e)
    );
  },

  setBubbleStyle: (style) => {
    set({ bubbleStyle: style });
    updateSetting("bubble_style", style).catch((e) =>
      console.error("Failed to save bubble_style:", e)
    );
  },

  setBubblePosition: (position) => {
    set({ bubblePosition: position });
    updateSetting("bubble_position", position).catch((e) =>
      console.error("Failed to save bubble_position:", e)
    );
  },

  setDefaultAction: (action) => {
    set({ defaultAction: action });
    updateSetting("default_action", action).catch((e) =>
      console.error("Failed to save default_action:", e)
    );
  },

  setSensitiveFilterEnabled: (enabled) => {
    set({ sensitiveFilterEnabled: enabled });
    updateSetting("sensitive_filter_enabled", String(enabled)).catch((e) =>
      console.error("Failed to save sensitive_filter_enabled:", e)
    );
  },


  setUrlReadingEnabled: (enabled) => {
    set({ urlReadingEnabled: enabled });
    updateSetting("url_reading_enabled", String(enabled)).catch((e) =>
      console.error("Failed to save url_reading_enabled:", e)
    );
  },

  setRadarIntervalDays: (days) => {
    set({ radarIntervalDays: days });
    updateSetting("radar_interval_days", String(days)).catch((e) =>
      console.error("Failed to save radar_interval_days:", e)
    );
  },

  setCountdownDuration: (seconds) => {
    set({ countdownDuration: seconds });
    updateSetting("countdown_seconds", String(seconds)).catch((e) =>
      console.error("Failed to save countdown_seconds:", e)
    );
  },

  setScreenshotDir: (dir) => set({ screenshotDir: dir }),
  setStorageInfo: (totalItems, diskUsageMB) =>
    set({ totalItems, diskUsageMB }),

  loadXReaderStatus: async () => {
    try {
      const status = await checkXReaderStatus();
      set({ xreaderStatus: status });
    } catch (e) {
      console.error("Failed to load x-reader status:", e);
    }
  },

  loadOAuthStatus: async () => {
    try {
      const status = await invoke<{ logged_in: boolean; email?: string }>("get_openai_oauth_status");
      set({ oauthLoggedIn: status.logged_in, oauthEmail: status.email || "" });
    } catch {
      set({ oauthLoggedIn: false, oauthEmail: "" });
    }
  },

  startOAuthLogin: async () => {
    set({ oauthLoading: true });
    try {
      const status = await invoke<{ logged_in: boolean; email?: string }>("start_openai_oauth");
      set({ oauthLoggedIn: status.logged_in, oauthEmail: status.email || "", oauthLoading: false });
    } catch (e) {
      set({ oauthLoading: false });
      throw e;
    }
  },

  logoutOAuth: async () => {
    try {
      await invoke("logout_openai_oauth");
      set({ oauthLoggedIn: false, oauthEmail: "" });
    } catch (e) {
      console.error("Logout failed:", e);
    }
  },

  loadGeminiOAuthStatus: async () => {
    try {
      const status = await invoke<{ logged_in: boolean; email?: string }>("get_gemini_oauth_status");
      set({ geminiOauthLoggedIn: status.logged_in, geminiOauthEmail: status.email || "" });
    } catch {
      set({ geminiOauthLoggedIn: false, geminiOauthEmail: "" });
    }
  },

  startGeminiOAuthLogin: async () => {
    set({ geminiOauthLoading: true });
    try {
      const status = await invoke<{ logged_in: boolean; email?: string }>("start_gemini_oauth");
      set({ geminiOauthLoggedIn: status.logged_in, geminiOauthEmail: status.email || "", geminiOauthLoading: false });
    } catch (e) {
      set({ geminiOauthLoading: false });
      throw e;
    }
  },

  logoutGeminiOAuth: async () => {
    try {
      await invoke("logout_gemini_oauth");
      set({ geminiOauthLoggedIn: false, geminiOauthEmail: "" });
    } catch (e) {
      console.error("Gemini logout failed:", e);
    }
  },
}));
