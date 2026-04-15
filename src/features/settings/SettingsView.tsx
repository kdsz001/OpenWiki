import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openExternal } from "@tauri-apps/plugin-shell";
import {
  Palette,
  Bot,
  Camera,
  Link as LinkIcon,
  HardDrive,
  Target,
  Info,
  RefreshCcw,
  CheckCircle2,
  ExternalLink,
  Stethoscope,
  ShieldCheck,
  ShieldAlert,
  ShieldQuestion,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  checkForUpdateManual,
  getUpdateSettings,
  setUpdateCheckEnabled,
  type UpdateInfo,
  type UpdateSettings,
} from "../../services/updateService";
import {
  getAutomationStatus,
  openAutomationSettings,
  type AutomationSnapshot,
} from "../../services/automationService";
import {
  useSettingsStore,
  MODELS_BY_PROVIDER,
  PROVIDER_LABELS,
  type AIProvider,
  type ThemeMode,
  type BubblePosition,
  type LanguageMode,
} from "../../stores/settingsStore";

const BUBBLE_POSITION_KEYS: { value: BubblePosition; key: string; icon: string }[] = [
  { value: "bottom-right", key: "capture.positions.bottom-right", icon: "↘" },
  { value: "bottom-center", key: "capture.positions.bottom-center", icon: "↓" },
  { value: "bottom-left", key: "capture.positions.bottom-left", icon: "↙" },
  { value: "top-right", key: "capture.positions.top-right", icon: "↗" },
  { value: "top-center", key: "capture.positions.top-center", icon: "↑" },
  { value: "top-left", key: "capture.positions.top-left", icon: "↖" },
];

const THEME_OPTIONS: { value: ThemeMode; key: string; icon: string }[] = [
  { value: "light", key: "theme.light", icon: "☀️" },
  { value: "dark", key: "theme.dark", icon: "🌙" },
  { value: "system", key: "theme.system", icon: "💻" },
];

const LANGUAGE_OPTIONS: { value: LanguageMode; key: string }[] = [
  { value: "system", key: "language.system" },
  { value: "zh-CN", key: "language.zh-CN" },
  { value: "en-US", key: "language.en-US" },
];

export function SettingsView() {
  const { t } = useTranslation("settings");
  const { t: tUpdate } = useTranslation("update");
  const { t: tAuto } = useTranslation("automation");
  const {
    apiKey,
    provider,
    model,
    theme,
    languageMode,
    captureEnabled,
    captureMode,
    bubbleStyle,
    bubblePosition,
    countdownDuration,
    sensitiveFilterEnabled,
    urlReadingEnabled,
    radarIntervalDays,
    screenshotDir,
    totalItems,
    diskUsageMB,
    storageMode,
    workspaceRoot,
    reopenWorkspaceSetup,
    setApiKey,
    setProvider,
    setModel,
    setTheme,
    setLanguageMode,
    setCaptureEnabled,
    setCaptureMode,
    setBubbleStyle,
    setBubblePosition,
    setCountdownDuration,
    setSensitiveFilterEnabled,
    defaultAction,
    setDefaultAction,
    setUrlReadingEnabled,
    setRadarIntervalDays,
    loadXReaderStatus,
    oauthLoggedIn,
    oauthEmail,
    oauthLoading,
    startOAuthLogin,
    logoutOAuth,
    geminiOauthLoggedIn,
    geminiOauthEmail,
    geminiOauthLoading,
    startGeminiOAuthLogin,
    logoutGeminiOAuth,
  } = useSettingsStore();

  const [showApiKey, setShowApiKey] = useState(false);
  const [draftApiKey, setDraftApiKey] = useState<string | null>(null);
  const [apiKeySaved, setApiKeySaved] = useState(false);
  const [testStatus, setTestStatus] = useState<"idle" | "testing" | "success" | "error">("idle");
  const [testMessage, setTestMessage] = useState("");
  // MCP connection state per target
  type McpTargetId = "claude" | "openclaw";
  interface McpTargetState {
    connected: boolean;
    loading: boolean;
    message: string | null;
    error: string | null;
  }
  const [mcpStates, setMcpStates] = useState<Record<McpTargetId, McpTargetState>>({
    claude: { connected: false, loading: false, message: null, error: null },
    openclaw: { connected: false, loading: false, message: null, error: null },
  });
  const [summaryCopied, setSummaryCopied] = useState(false);
  const [mcpGlobalError, setMcpGlobalError] = useState<string | null>(null);

  const updateMcpTarget = (id: McpTargetId, update: Partial<McpTargetState>) => {
    setMcpStates((prev) => ({ ...prev, [id]: { ...prev[id], ...update } }));
  };

  const loadMcpStatus = useCallback(async () => {
    for (const target of ["claude", "openclaw"] as McpTargetId[]) {
      try {
        const status = await invoke<{ connected: boolean }>("get_mcp_status", { target });
        updateMcpTarget(target, { connected: status.connected });
      } catch {
        // silently fail — target may not be installed
      }
    }
  }, []);

  useEffect(() => {
    loadMcpStatus();
  }, [loadMcpStatus]);

  const handleConnectMcp = async (target: McpTargetId) => {
    updateMcpTarget(target, { loading: true, error: null, message: null });
    try {
      const msg = await invoke<string>("connect_mcp", { target });
      updateMcpTarget(target, { loading: false, message: msg, connected: true });
    } catch (e) {
      const errMsg = typeof e === "string" ? e : String(e);
      console.error("[MCP] connect error:", errMsg);
      updateMcpTarget(target, { loading: false, error: errMsg });
    }
  };

  const handleDisconnectMcp = async (target: McpTargetId) => {
    updateMcpTarget(target, { loading: true, error: null, message: null });
    try {
      await invoke("disconnect_mcp", { target });
      updateMcpTarget(target, { loading: false, connected: false, message: t("connection.disconnectedMsg") });
    } catch (e) {
      updateMcpTarget(target, { loading: false, error: typeof e === "string" ? e : String(e) });
    }
  };


  const { setStorageInfo } = useSettingsStore();

  useEffect(() => {
    loadXReaderStatus();
    // Load storage info
    invoke<{ total_items: number; disk_usage_mb: number }>("get_storage_info")
      .then((info) => setStorageInfo(info.total_items, info.disk_usage_mb))
      .catch(() => {});
  }, [loadXReaderStatus, setStorageInfo]);

  const categories = [
    { id: "appearance", label: t("sections.appearance"), icon: Palette },
    { id: "capture", label: t("sections.capture"), icon: Camera },
    { id: "radar", label: t("sections.insights"), icon: Target },
    { id: "ai", label: t("sections.ai"), icon: Bot },
    { id: "connect", label: t("sections.connection"), icon: LinkIcon },
    { id: "storage", label: t("sections.storage"), icon: HardDrive },
    { id: "about", label: tUpdate("settings.sectionTitle"), icon: Info },
    { id: "diagnostics", label: tAuto("settings.sectionTitle"), icon: Stethoscope },
  ];
  const [activeCategory, setActiveCategory] = useState("appearance");

  // ===== Automation permission state =====
  const [automationSnapshot, setAutomationSnapshot] =
    useState<AutomationSnapshot | null>(null);

  const refreshAutomation = useCallback(async () => {
    try {
      setAutomationSnapshot(await getAutomationStatus());
    } catch (e) {
      console.error("[automation] failed to load status:", e);
    }
  }, []);

  useEffect(() => {
    refreshAutomation();
  }, [refreshAutomation]);

  // Re-read on grant/deny events so the diagnostics pane stays fresh
  // even when the user changed permission from System Settings mid-session.
  useEffect(() => {
    const handler = () => refreshAutomation();
    window.addEventListener("automation-granted", handler);
    window.addEventListener("automation-denied", handler);
    return () => {
      window.removeEventListener("automation-granted", handler);
      window.removeEventListener("automation-denied", handler);
    };
  }, [refreshAutomation]);

  const handleRequestAutomation = () => {
    // Reuses the same modal users see on first launch.
    window.dispatchEvent(new CustomEvent("automation-needed-manual"));
  };

  const handleOpenSystemSettings = async () => {
    try {
      await openAutomationSettings();
    } catch (e) {
      console.error("[automation] open settings failed:", e);
    }
  };

  // ===== Update check state =====
  const [updateSettings, setUpdateSettingsState] = useState<UpdateSettings | null>(null);
  const [checking, setChecking] = useState(false);
  const [latestInfo, setLatestInfo] = useState<UpdateInfo | null>(null);
  const [checkResult, setCheckResult] = useState<"up-to-date" | "error" | null>(null);
  const [checkError, setCheckError] = useState<string>("");

  // Load update settings once (current version + auto-check toggle state)
  useEffect(() => {
    getUpdateSettings()
      .then(setUpdateSettingsState)
      .catch((e) => console.error("[update] failed to load settings:", e));
  }, []);

  const handleCheckNow = async () => {
    setChecking(true);
    setCheckResult(null);
    setCheckError("");
    try {
      const info = await checkForUpdateManual();
      if (info) {
        setLatestInfo(info);
        // Ask the top-level UpdateBanner to render as well, for consistency
        // with what the user sees from the background startup check.
        window.dispatchEvent(
          new CustomEvent<UpdateInfo>("update-available-manual", { detail: info }),
        );
      } else {
        setLatestInfo(null);
        setCheckResult("up-to-date");
      }
    } catch (e) {
      setCheckResult("error");
      setCheckError(String(e));
    } finally {
      setChecking(false);
    }
  };

  const handleToggleAutoCheck = async (enabled: boolean) => {
    try {
      await setUpdateCheckEnabled(enabled);
      setUpdateSettingsState((prev) =>
        prev ? { ...prev, check_enabled: enabled } : prev,
      );
    } catch (e) {
      console.error("[update] failed to toggle auto-check:", e);
    }
  };

  const handleOpenReleases = async () => {
    if (!updateSettings) return;
    try {
      await openExternal(updateSettings.releases_url);
    } catch (e) {
      console.error("[update] failed to open releases page:", e);
    }
  };

  return (
    <div className="flex" style={{ height: "calc(100vh - 44px)" }}>
      {/* Left: category nav */}
      <div
        className="w-36 shrink-0 px-2 overflow-y-auto border-r flex flex-col"
        style={{ borderColor: "var(--color-border, #e5e5e5)" }}
      >
        <div className="flex-1 pt-2">
          {categories.map((cat) => {
            const Icon = cat.icon;
            const isActive = activeCategory === cat.id;
            return (
              <button
                key={cat.id}
                onClick={() => setActiveCategory(cat.id)}
                className={`
                  w-full flex items-center gap-2 px-3 py-2 rounded-lg text-sm font-medium mb-1 transition-colors
                  ${isActive
                    ? "bg-orange-500/10 dark:bg-orange-500/15 text-orange-600 dark:text-orange-400"
                    : "text-gray-600 dark:text-gray-400 hover:bg-gray-100/50 dark:hover:bg-white/[0.04]"
                  }
                `}
              >
                <Icon size={16} strokeWidth={2} />
                {cat.label}
              </button>
            );
          })}
        </div>
        <div className="py-3 px-3">
          <p className="text-[10px] text-gray-400 dark:text-gray-600">
            OpenWiki v{updateSettings?.current_version ?? "…"}
          </p>
        </div>
      </div>

      {/* Right: settings content */}
      <div className="flex-1 overflow-y-auto p-6 flex justify-center">
        <div className="w-full max-w-xl">

      {/* ===== Appearance ===== */}
      {activeCategory === "appearance" && (
        <div className="space-y-6">
          <h2 className="text-lg font-semibold text-gray-800 dark:text-gray-100">{t("sections.appearance")}</h2>
          <div className="glass rounded-2xl">
            {/* Theme */}
            <div className="p-4">
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-2">
                {t("theme.label")}
              </label>
              <div className="flex gap-2">
                {THEME_OPTIONS.map((opt) => (
                  <button
                    key={opt.value}
                    onClick={() => setTheme(opt.value)}
                    className={`
                      flex-1 flex items-center justify-center gap-1.5 px-3 py-2.5 text-sm font-medium rounded-lg border transition-all duration-150
                      ${theme === opt.value
                        ? "bg-orange-500/10 dark:bg-orange-500/15 border-orange-300/60 dark:border-orange-500/30 text-orange-700 dark:text-orange-400 shadow-sm"
                        : "bg-white/50 dark:bg-white/[0.04] border-white/60 dark:border-white/[0.08] text-gray-600 dark:text-slate-300 hover:bg-white/80 dark:hover:bg-white/[0.08]"
                      }
                    `}
                  >
                    <span>{opt.icon}</span>
                    <span>{t(opt.key)}</span>
                  </button>
                ))}
              </div>
            </div>
            {/* Language */}
            <div className="p-4 border-t border-gray-100/50 dark:border-white/[0.06]">
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                {t("language.label")}
              </label>
              <p className="text-xs text-gray-400 dark:text-slate-500 mb-2">{t("language.description")}</p>
              <div className="flex gap-2">
                {LANGUAGE_OPTIONS.map((opt) => (
                  <button
                    key={opt.value}
                    onClick={() => setLanguageMode(opt.value)}
                    className={`
                      flex-1 flex items-center justify-center gap-1.5 px-3 py-2.5 text-sm font-medium rounded-lg border transition-all duration-150
                      ${languageMode === opt.value
                        ? "bg-orange-500/10 dark:bg-orange-500/15 border-orange-300/60 dark:border-orange-500/30 text-orange-700 dark:text-orange-400 shadow-sm"
                        : "bg-white/50 dark:bg-white/[0.04] border-white/60 dark:border-white/[0.08] text-gray-600 dark:text-slate-300 hover:bg-white/80 dark:hover:bg-white/[0.08]"
                      }
                    `}
                  >
                    <span>{t(opt.key)}</span>
                  </button>
                ))}
              </div>
            </div>
          </div>
        </div>
      )}

      {/* ===== Capture ===== */}
      {activeCategory === "capture" && (
        <div className="space-y-1">
          <h2 className="text-lg font-semibold text-gray-800 dark:text-gray-100 mb-4">{t("sections.capture")}</h2>
          <div className="glass rounded-2xl divide-y divide-gray-100/50 dark:divide-white/[0.06]">

          {/* Capture Toggle */}
          <SettingRow label={t("capture.enabled")} desc={t("capture.enabledDesc")}>
            <ToggleSwitch checked={captureEnabled} onChange={setCaptureEnabled} color="orange" />
          </SettingRow>

          {/* Capture Mode */}
          <SettingRow label={t("capture.mode")} desc={t("capture.modeDesc")}>
            <div className="flex gap-1.5">
              {([
                { value: "confirm", key: "capture.confirm" },
                { value: "auto", key: "capture.auto" },
              ] as const).map((opt) => (
                <button
                  key={opt.value}
                  onClick={() => setCaptureMode(opt.value)}
                  className={`px-3 py-1.5 text-xs font-medium rounded-lg border transition-colors
                    ${captureMode === opt.value
                      ? "bg-orange-500/10 dark:bg-orange-500/15 border-orange-300/60 dark:border-orange-500/30 text-orange-700 dark:text-orange-400"
                      : "bg-white/50 dark:bg-white/[0.04] border-gray-200/50 dark:border-white/[0.08] text-gray-600 dark:text-slate-300"
                    }`}
                >
                  {t(opt.key)}
                </button>
              ))}
            </div>
          </SettingRow>

          {/* Default Action */}
          {captureMode === "confirm" && (
            <SettingRow label={t("capture.defaultAction")} desc={t("capture.defaultActionDesc")}>
              <div className="flex gap-1.5">
                {([
                  { value: "dismiss", key: "capture.defaultDismiss" },
                  { value: "save", key: "capture.defaultSave" },
                ] as const).map((opt) => (
                  <button
                    key={opt.value}
                    onClick={() => setDefaultAction(opt.value)}
                    className={`px-3 py-1.5 text-xs font-medium rounded-lg border transition-colors
                      ${defaultAction === opt.value
                        ? "bg-orange-500/10 dark:bg-orange-500/15 border-orange-300/60 dark:border-orange-500/30 text-orange-700 dark:text-orange-400"
                        : "bg-white/50 dark:bg-white/[0.04] border-gray-200/50 dark:border-white/[0.08] text-gray-600 dark:text-slate-300"
                      }`}
                  >
                    {t(opt.key)}
                  </button>
                ))}
              </div>
            </SettingRow>
          )}

          {/* Bubble Style */}
          {captureMode === "confirm" && (
            <SettingRow label={t("capture.bubbleStyle")}>
              <div className="flex gap-1.5">
                {([
                  { value: "circle", key: "capture.circle" },
                  { value: "bar", key: "capture.bar" },
                ] as const).map((opt) => (
                  <button
                    key={opt.value}
                    onClick={() => setBubbleStyle(opt.value)}
                    className={`px-3 py-1.5 text-xs font-medium rounded-lg border transition-colors
                      ${bubbleStyle === opt.value
                        ? "bg-orange-500/10 dark:bg-orange-500/15 border-orange-300/60 dark:border-orange-500/30 text-orange-700 dark:text-orange-400"
                        : "bg-white/50 dark:bg-white/[0.04] border-gray-200/50 dark:border-white/[0.08] text-gray-600 dark:text-slate-300"
                      }`}
                  >
                    {t(opt.key)}
                  </button>
                ))}
              </div>
            </SettingRow>
          )}

          {/* Bubble Position */}
          {captureMode === "confirm" && (
            <SettingRow label={t("capture.bubblePosition")}>
              <select
                value={bubblePosition}
                onChange={(e) => setBubblePosition(e.target.value as BubblePosition)}
                className="text-sm rounded-lg px-3 py-1.5 bg-white/40 dark:bg-white/[0.06] border border-gray-200/50 dark:border-white/[0.08] text-gray-700 dark:text-gray-300 focus:outline-none focus:ring-1 focus:ring-orange-400/50"
              >
                {BUBBLE_POSITION_KEYS.map((opt) => (
                  <option key={opt.value} value={opt.value}>{opt.icon} {t(opt.key)}</option>
                ))}
              </select>
            </SettingRow>
          )}

          {/* Countdown */}
          {captureMode === "confirm" && (
            <SettingRow label={t("capture.countdown")} desc={t("capture.countdownDesc")}>
              <select
                value={countdownDuration}
                onChange={(e) => setCountdownDuration(Number(e.target.value))}
                className="text-sm rounded-lg px-3 py-1.5 bg-white/40 dark:bg-white/[0.06] border border-gray-200/50 dark:border-white/[0.08] text-gray-700 dark:text-gray-300 focus:outline-none focus:ring-1 focus:ring-orange-400/50"
              >
                {[3, 5, 8, 10, 15].map((s) => (
                  <option key={s} value={s}>{s} {t("capture.countdownUnit")}</option>
                ))}
              </select>
            </SettingRow>
          )}

          {/* Sensitive Filter */}
          <SettingRow label={t("capture.sensitiveFilter")} desc={t("capture.sensitiveFilterDesc")}>
            <ToggleSwitch checked={sensitiveFilterEnabled} onChange={setSensitiveFilterEnabled} color="amber" />
          </SettingRow>

          {/* URL Reading */}
          <SettingRow label={t("capture.urlReading")} desc={t("capture.urlReadingDesc")}>
            <ToggleSwitch checked={urlReadingEnabled} onChange={setUrlReadingEnabled} color="green" />
          </SettingRow>

          </div>
        </div>
      )}

      {/* ===== Insights ===== */}
      {activeCategory === "radar" && (
        <div className="space-y-1">
          <h2 className="text-lg font-semibold text-gray-800 dark:text-gray-100 mb-4">{t("insights.title")}</h2>
          <div className="glass rounded-2xl divide-y divide-gray-100/50 dark:divide-white/[0.06]">
            <SettingRow label={t("insights.interval")} desc={t("insights.intervalDesc")}>
              <select
                value={radarIntervalDays}
                onChange={(e) => setRadarIntervalDays(Number(e.target.value))}
                className="text-sm rounded-lg px-3 py-1.5 bg-white/40 dark:bg-white/[0.06] border border-gray-200/50 dark:border-white/[0.08] text-gray-700 dark:text-gray-300 focus:outline-none focus:ring-1 focus:ring-orange-400/50"
              >
                <option value={1}>{t("insights.intervalDaily")}</option>
                <option value={3}>{t("insights.interval3Days")}</option>
                <option value={7}>{t("insights.intervalWeekly")}</option>
                <option value={30}>{t("insights.intervalMonthly")}</option>
              </select>
            </SettingRow>
          </div>
          <p className="text-xs text-gray-400 dark:text-gray-600 mt-3 px-1">
            {t("insights.hint")}
          </p>
        </div>
      )}

      {/* ===== AI ===== */}
      {activeCategory === "ai" && (
        <div className="space-y-1">
          <h2 className="text-lg font-semibold text-gray-800 dark:text-gray-100 mb-4">{t("ai.title")}</h2>
          <div className="glass rounded-2xl divide-y divide-gray-100/50 dark:divide-white/[0.06]">

          {/* Provider */}
          <SettingRow label={t("ai.provider")}>
            <select
              value={provider}
              onChange={(e) => {
                setProvider(e.target.value as AIProvider);
                setDraftApiKey(null);
                setTestStatus("idle");
                setTestMessage("");
                setApiKeySaved(false);
              }}
              className="text-sm rounded-lg px-3 py-1.5 bg-white/40 dark:bg-white/[0.06] border border-gray-200/50 dark:border-white/[0.08] text-gray-700 dark:text-gray-300 focus:outline-none focus:ring-1 focus:ring-orange-400/50"
            >
              {(Object.entries(PROVIDER_LABELS) as [AIProvider, string][]).map(([value, label]) => (
                <option key={value} value={value}>{label}</option>
              ))}
            </select>
          </SettingRow>

          {/* Model */}
          <SettingRow label={t("ai.model")}>
            <select
              value={model}
              onChange={(e) => setModel(e.target.value)}
              className="text-sm rounded-lg px-3 py-1.5 bg-white/40 dark:bg-white/[0.06] border border-gray-200/50 dark:border-white/[0.08] text-gray-700 dark:text-gray-300 focus:outline-none focus:ring-1 focus:ring-orange-400/50 max-w-[220px]"
            >
              {MODELS_BY_PROVIDER[provider].map((m) => (
                <option key={m.id} value={m.id}>
                  {m.free ? "🆓 " : ""}{m.label}
                </option>
              ))}
            </select>
          </SettingRow>

          {/* OpenAI OAuth Login */}
          {provider === "openai" && (
            <div className="p-4">
              <div className="flex items-center justify-between mb-2">
                <div>
                  <div className="text-sm font-medium text-gray-700 dark:text-gray-300">{t("ai.oauthTitle")}</div>
                  <div className="text-xs text-gray-400 dark:text-slate-500 mt-0.5">{t("ai.oauthOpenAIDesc")}</div>
                </div>
              </div>
              {oauthLoggedIn ? (
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <span className="text-sm text-green-600 dark:text-green-400">{t("ai.oauthLoggedIn")}</span>
                    <span className="text-xs text-gray-400 dark:text-gray-500">{oauthEmail}</span>
                  </div>
                  <button
                    onClick={logoutOAuth}
                    className="px-3 py-1.5 text-xs font-medium rounded-lg border border-gray-200/50 dark:border-white/[0.08] text-gray-500 dark:text-slate-400 hover:bg-gray-100/50 dark:hover:bg-white/[0.04] transition-colors"
                  >
                    {t("ai.oauthLogout")}
                  </button>
                </div>
              ) : (
                <div>
                  <button
                    onClick={async () => {
                      try {
                        await startOAuthLogin();
                      } catch (e) {
                        alert(typeof e === "string" ? e : t("ai.oauthLoginFailed"));
                      }
                    }}
                    disabled={oauthLoading}
                    className="w-full px-4 py-2.5 text-sm font-medium rounded-lg bg-[#10a37f] hover:bg-[#0d8c6d] text-white transition-colors disabled:opacity-50 disabled:cursor-default"
                  >
                    {oauthLoading ? t("ai.oauthLoading") : t("ai.oauthLoginOpenAI")}
                  </button>
                  <p className="text-xs text-gray-400 dark:text-gray-600 mt-2">
                    {t("ai.oauthOpenAIHint")}
                  </p>
                </div>
              )}
            </div>
          )}

          {/* Google OAuth Login */}
          {provider === "google" && (
            <div className="p-4">
              <div className="flex items-center justify-between mb-2">
                <div>
                  <div className="text-sm font-medium text-gray-700 dark:text-gray-300">{t("ai.oauthTitle")}</div>
                  <div className="text-xs text-gray-400 dark:text-slate-500 mt-0.5">{t("ai.oauthGeminiDesc")}</div>
                </div>
              </div>
              {geminiOauthLoggedIn ? (
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <span className="text-sm text-green-600 dark:text-green-400">{t("ai.oauthLoggedIn")}</span>
                    <span className="text-xs text-gray-400 dark:text-gray-500">{geminiOauthEmail}</span>
                  </div>
                  <button
                    onClick={logoutGeminiOAuth}
                    className="px-3 py-1.5 text-xs font-medium rounded-lg border border-gray-200/50 dark:border-white/[0.08] text-gray-500 dark:text-slate-400 hover:bg-gray-100/50 dark:hover:bg-white/[0.04] transition-colors"
                  >
                    {t("ai.oauthLogout")}
                  </button>
                </div>
              ) : (
                <div>
                  <button
                    onClick={async () => {
                      try { await startGeminiOAuthLogin(); }
                      catch (e) { alert(typeof e === "string" ? e : t("ai.oauthLoginFailed")); }
                    }}
                    disabled={geminiOauthLoading}
                    className="w-full px-4 py-2.5 text-sm font-medium rounded-lg bg-[#4285f4] hover:bg-[#3367d6] text-white transition-colors disabled:opacity-50 disabled:cursor-default"
                  >
                    {geminiOauthLoading ? t("ai.oauthLoading") : t("ai.oauthLoginGoogle")}
                  </button>
                  <p className="text-xs text-gray-400 dark:text-gray-600 mt-2">
                    {t("ai.oauthGeminiHint")}
                  </p>
                </div>
              )}
            </div>
          )}

          {/* API Key */}
          <div className="p-4">
            <div className="flex items-center justify-between mb-2">
              <div>
                <div className="text-sm font-medium text-gray-700 dark:text-gray-300">{t("ai.apiKey")}</div>
                <div className="text-xs text-gray-400 dark:text-slate-500 mt-0.5">{t("ai.apiKeyDesc")}</div>
              </div>
            </div>
            <div className="flex gap-2">
              <input
                type={showApiKey ? "text" : "password"}
                value={draftApiKey ?? apiKey}
                onChange={(e) => {
                  setDraftApiKey(e.target.value);
                  setApiKeySaved(false);
                  setTestStatus("idle");
                }}
                placeholder={t("ai.apiKeyPlaceholder")}
                className="flex-1 px-3 py-2 text-sm rounded-lg bg-white/50 dark:bg-white/[0.04] border border-gray-200/50 dark:border-white/[0.08] text-gray-800 dark:text-gray-200 placeholder-gray-400 dark:placeholder-slate-600 focus:outline-none focus:ring-1 focus:ring-orange-400/50"
              />
              <button
                onClick={() => setShowApiKey(!showApiKey)}
                className="px-3 py-2 text-xs font-medium rounded-lg border border-gray-200/50 dark:border-white/[0.08] text-gray-500 dark:text-slate-400 hover:bg-gray-100/50 dark:hover:bg-white/[0.04] transition-colors"
              >
                {showApiKey ? t("ai.apiKeyHide") : t("ai.apiKeyShow")}
              </button>
            </div>
            <div className="flex gap-2 mt-2">
              <button
                onClick={() => {
                  const key = draftApiKey ?? apiKey;
                  setApiKey(key);
                  setDraftApiKey(null);
                  setApiKeySaved(true);
                  setTimeout(() => setApiKeySaved(false), 2000);
                }}
                disabled={draftApiKey === null || draftApiKey === apiKey}
                className="px-4 py-1.5 text-xs font-medium rounded-lg border transition-colors
                  disabled:opacity-30 disabled:cursor-default
                  bg-orange-500/10 dark:bg-orange-500/15 border-orange-300/60 dark:border-orange-500/30 text-orange-700 dark:text-orange-400 hover:bg-orange-500/20 dark:hover:bg-orange-500/25"
              >
                {apiKeySaved ? t("ai.apiKeySaved") : t("ai.apiKeySave")}
              </button>
              <button
                onClick={async () => {
                  const key = draftApiKey ?? apiKey;
                  if (!key) return;
                  // Save first if draft exists
                  if (draftApiKey !== null && draftApiKey !== apiKey) {
                    setApiKey(draftApiKey);
                    setDraftApiKey(null);
                  }
                  setTestStatus("testing");
                  setTestMessage("");
                  try {
                    const result = await invoke<string>("test_ai_connection", {
                      provider, model, apiKey: key,
                    });
                    setTestStatus("success");
                    setTestMessage(result);
                  } catch (e) {
                    setTestStatus("error");
                    setTestMessage(typeof e === "string" ? e : String(e));
                  }
                }}
                disabled={!(draftApiKey ?? apiKey) || testStatus === "testing"}
                className="px-4 py-1.5 text-xs font-medium rounded-lg border transition-colors
                  disabled:opacity-30 disabled:cursor-default
                  bg-white/50 dark:bg-white/[0.04] border-gray-200/50 dark:border-white/[0.08] text-gray-600 dark:text-slate-300 hover:bg-white/80 dark:hover:bg-white/[0.08]"
              >
                {testStatus === "testing" ? t("ai.testing") : t("ai.testConnection")}
              </button>
            </div>
            {testStatus === "success" && (
              <p className="mt-2 text-xs text-green-600 dark:text-green-400">{t("ai.testSuccess", { message: testMessage })}</p>
            )}
            {testStatus === "error" && (
              <p className="mt-2 text-xs text-red-500 dark:text-red-400">{t("ai.testFailed", { message: testMessage })}</p>
            )}
          </div>

          </div>
        </div>
      )}

      {/* ===== Connection ===== */}
      {activeCategory === "connect" && (
        <div className="space-y-1">
          <h2 className="text-lg font-semibold text-gray-800 dark:text-gray-100 mb-4">{t("connection.title")}</h2>
          <div className="glass rounded-2xl divide-y divide-gray-100/50 dark:divide-white/[0.06]">
          {([
            { id: "claude" as McpTargetId, name: "Claude Desktop" },
          ]).map((tgt) => {
            const s = mcpStates[tgt.id];
            return (
              <div key={tgt.id} className="p-4">
                <div className="flex items-center justify-between mb-2">
                  <div>
                    <div className="text-sm font-medium text-gray-700 dark:text-gray-300">
                      {tgt.name}
                    </div>
                    <div className="text-xs text-gray-400 dark:text-slate-500 mt-0.5">
                      {s.connected
                        ? t("connection.connectedHint", { name: tgt.name })
                        : t("connection.disconnectedHint", { name: tgt.name })}
                    </div>
                  </div>
                  <div className="flex items-center gap-2">
                    <span className={`w-2 h-2 rounded-full ${s.connected ? "bg-green-500" : "bg-gray-300 dark:bg-slate-600"}`} />
                    <span className="text-xs text-gray-500 dark:text-slate-400">
                      {s.connected ? t("connection.connected") : t("connection.disconnected")}
                    </span>
                  </div>
                </div>

                {s.connected ? (
                  <button
                    onClick={() => handleDisconnectMcp(tgt.id)}
                    disabled={s.loading}
                    className="w-full py-2 text-sm font-medium rounded-lg border text-red-500 dark:text-red-400 border-red-200/50 dark:border-red-500/20 bg-red-50/50 dark:bg-red-500/[0.06] hover:bg-red-100/50 dark:hover:bg-red-500/[0.12] disabled:opacity-50 transition-colors"
                  >
                    {s.loading ? t("connection.disconnecting") : t("connection.disconnectBtn")}
                  </button>
                ) : (
                  <button
                    onClick={() => handleConnectMcp(tgt.id)}
                    disabled={s.loading}
                    className="w-full py-2 text-sm font-medium rounded-lg border text-orange-600 dark:text-orange-400 border-orange-200/50 dark:border-orange-500/20 bg-orange-50/50 dark:bg-orange-500/[0.06] hover:bg-orange-100/50 dark:hover:bg-orange-500/[0.12] disabled:opacity-50 transition-colors"
                  >
                    {s.loading ? t("connection.connecting") : t("connection.connectBtn", { name: tgt.name })}
                  </button>
                )}

                {s.message && <p className="mt-2 text-xs text-green-600 dark:text-green-400">{s.message}</p>}
                {s.error && <p className="mt-2 text-xs text-red-500 dark:text-red-400">{s.error}</p>}
              </div>
            );
          })}

          {/* Copy Summary */}
          <div className="p-4">
            <div className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              {t("connection.summaryTitle")}
            </div>
            <div className="text-xs text-gray-400 dark:text-slate-500 mb-3">
              {t("connection.summaryDesc")}
            </div>
            <button
              onClick={async () => {
                try {
                  await invoke("copy_content_summary");
                  setSummaryCopied(true);
                  setTimeout(() => setSummaryCopied(false), 3000);
                } catch (e) {
                  setMcpGlobalError(typeof e === "string" ? e : String(e));
                }
              }}
              className="w-full py-2 text-sm font-medium rounded-lg border text-gray-600 dark:text-gray-300 border-gray-200/50 dark:border-white/[0.08] bg-white/40 dark:bg-white/[0.04] hover:bg-white/70 dark:hover:bg-white/[0.08] transition-colors"
            >
              {summaryCopied ? t("connection.summaryCopied") : t("connection.summaryCopyBtn")}
            </button>
            {mcpGlobalError && <p className="mt-2 text-xs text-red-500 dark:text-red-400">{mcpGlobalError}</p>}
          </div>
          </div>
        </div>
      )}

      {/* ===== Storage ===== */}
      {activeCategory === "storage" && (
        <div className="space-y-1">
          <h2 className="text-lg font-semibold text-gray-800 dark:text-gray-100 mb-4">{t("storage.title")}</h2>
          <div className="glass rounded-2xl divide-y divide-gray-100/50 dark:divide-white/[0.06]">
            <SettingRow label={t("storage.totalItems")}>
              <span className="text-sm font-mono text-gray-700 dark:text-gray-300">{totalItems} {t("storage.totalItemsUnit")}</span>
            </SettingRow>
            <SettingRow label={t("storage.diskUsage")}>
              <span className="text-sm font-mono text-gray-700 dark:text-gray-300">{diskUsageMB.toFixed(1)} {t("storage.unit")}</span>
            </SettingRow>
            <SettingRow label={t("storage.screenshotDir")}>
              <span className="text-xs font-mono text-gray-500 dark:text-slate-400 break-all">{screenshotDir}</span>
            </SettingRow>
            <SettingRow label={t("workspaceSetup.modeTitle")}>
              <span className="text-sm text-gray-700 dark:text-gray-300">
                {storageMode === "connected"
                  ? t("workspaceSetup.modeConnectedTitle")
                  : storageMode === "hybrid"
                    ? t("workspaceSetup.modeHybridTitle")
                    : t("workspaceSetup.modeManagedTitle")}
              </span>
            </SettingRow>
            <SettingRow
              label={t("workspaceSetup.workspaceRootTitle")}
              desc={t("workspaceSetup.workspaceRootDesc")}
            >
              <span className="text-xs font-mono text-gray-500 dark:text-slate-400 break-all">
                {workspaceRoot || "—"}
              </span>
            </SettingRow>
            <div className="p-4">
              <div className="flex flex-col gap-2">
                <button
                  onClick={() => invoke("open_data_folder").catch((e) => console.error("open_data_folder failed:", e))}
                  className="w-full py-2 text-sm font-medium rounded-lg border text-gray-600 dark:text-gray-300 border-gray-200/50 dark:border-white/[0.08] bg-white/40 dark:bg-white/[0.04] hover:bg-white/70 dark:hover:bg-white/[0.08] transition-colors"
                >
                  {t("storage.openDataFolder")}
                </button>
                <button
                  onClick={() => import("../../services/workspaceService")
                    .then((ws) => ws.openWorkspaceRoot())
                    .catch((e) => console.error("open_workspace_root failed:", e))}
                  className="w-full py-2 text-sm font-medium rounded-lg border text-gray-600 dark:text-gray-300 border-gray-200/50 dark:border-white/[0.08] bg-white/40 dark:bg-white/[0.04] hover:bg-white/70 dark:hover:bg-white/[0.08] transition-colors"
                >
                  {t("workspaceSetup.openWorkspace")}
                </button>
                <button
                  onClick={() => reopenWorkspaceSetup().catch((e) => console.error("reopenWorkspaceSetup failed:", e))}
                  className="w-full py-2 text-sm font-medium rounded-lg bg-orange-500 text-white hover:bg-orange-600 transition-colors"
                >
                  {t("workspaceSetup.reopenSetup")}
                </button>
              </div>
            </div>
          </div>

          {/* Export section */}
          <h2 className="text-lg font-semibold text-gray-800 dark:text-gray-100 mb-4 mt-6">{t("export.title")}</h2>
          <ExportSection totalItems={totalItems} />
        </div>
      )}

      {/* ===== About / Update ===== */}
      {activeCategory === "about" && (
        <div className="space-y-1">
          <h2 className="text-lg font-semibold text-gray-800 dark:text-gray-100 mb-1">
            {tUpdate("settings.sectionTitle")}
          </h2>
          <p className="text-xs text-gray-500 dark:text-slate-400 mb-4">
            {tUpdate("settings.sectionDescription")}
          </p>

          <div className="glass rounded-2xl divide-y divide-gray-100/50 dark:divide-white/[0.06]">
            <SettingRow label={tUpdate("settings.currentVersion")}>
              <span className="text-sm font-mono text-gray-700 dark:text-gray-300">
                v{updateSettings?.current_version ?? "…"}
              </span>
            </SettingRow>

            <SettingRow label={tUpdate("settings.latestVersion")}>
              {latestInfo ? (
                <span className="text-sm font-mono text-orange-600 dark:text-orange-400 font-semibold">
                  v{latestInfo.version}
                </span>
              ) : checkResult === "up-to-date" ? (
                <span className="inline-flex items-center gap-1 text-xs text-green-600 dark:text-green-400">
                  <CheckCircle2 className="w-3.5 h-3.5" />
                  {tUpdate("settings.upToDate")}
                </span>
              ) : (
                <span className="text-xs text-gray-400 dark:text-slate-500">—</span>
              )}
            </SettingRow>

            <SettingRow
              label={tUpdate("settings.autoCheckLabel")}
              desc={tUpdate("settings.autoCheckHint")}
            >
              <ToggleSwitch
                checked={updateSettings?.check_enabled ?? true}
                onChange={handleToggleAutoCheck}
              />
            </SettingRow>

            <div className="p-4 flex flex-col gap-2">
              <button
                onClick={handleCheckNow}
                disabled={checking}
                className="w-full flex items-center justify-center gap-2 py-2 text-sm font-medium rounded-lg
                           bg-orange-500 text-white hover:bg-orange-600
                           disabled:bg-gray-300 dark:disabled:bg-white/[0.06]
                           disabled:text-gray-400 dark:disabled:text-slate-500
                           disabled:cursor-not-allowed transition-colors"
              >
                <RefreshCcw className={`w-3.5 h-3.5 ${checking ? "animate-spin" : ""}`} />
                {checking ? tUpdate("settings.checking") : tUpdate("settings.checkNow")}
              </button>

              <button
                onClick={handleOpenReleases}
                className="w-full flex items-center justify-center gap-2 py-2 text-sm font-medium rounded-lg
                           border text-gray-600 dark:text-gray-300
                           border-gray-200/50 dark:border-white/[0.08]
                           bg-white/40 dark:bg-white/[0.04]
                           hover:bg-white/70 dark:hover:bg-white/[0.08] transition-colors"
              >
                <ExternalLink className="w-3.5 h-3.5" />
                {tUpdate("settings.viewReleases")}
              </button>

              {checkResult === "error" && (
                <p className="text-xs text-red-500 dark:text-red-400 mt-1 break-words">
                  {tUpdate("settings.checkFailed", { error: checkError })}
                </p>
              )}
            </div>
          </div>
        </div>
      )}

      {/* ===== Diagnostics (Automation permission) ===== */}
      {activeCategory === "diagnostics" && (
        <div className="space-y-1">
          <h2 className="text-lg font-semibold text-gray-800 dark:text-gray-100 mb-1">
            {tAuto("settings.sectionTitle")}
          </h2>
          <p className="text-xs text-gray-500 dark:text-slate-400 mb-4">
            {tAuto("settings.sectionDescription")}
          </p>

          <div className="glass rounded-2xl">
            <div className="p-5 flex items-start gap-4">
              {/* Status icon */}
              <div className="flex-shrink-0 mt-0.5">
                {automationSnapshot?.status === "granted" && (
                  <ShieldCheck className="w-8 h-8 text-green-500" />
                )}
                {automationSnapshot?.status === "denied" && (
                  <ShieldAlert className="w-8 h-8 text-red-500" />
                )}
                {(automationSnapshot?.status === "dismissed" ||
                  automationSnapshot?.status === "unknown" ||
                  !automationSnapshot) && (
                  <ShieldQuestion className="w-8 h-8 text-gray-400 dark:text-slate-500" />
                )}
              </div>

              <div className="flex-1 min-w-0">
                <div className="text-sm font-semibold text-gray-800 dark:text-gray-100">
                  {tAuto("settings.automationLabel")}
                </div>
                <div className="text-xs text-gray-500 dark:text-slate-400 mt-0.5 mb-3">
                  {tAuto("settings.automationDesc")}
                </div>

                <div className="mb-3">
                  {automationSnapshot?.status === "granted" && (
                    <span className="inline-flex items-center gap-1 px-2.5 py-1 rounded-full text-[11px] font-medium
                                     bg-green-500/10 text-green-600 dark:text-green-400
                                     border border-green-500/20">
                      {tAuto("settings.statusGranted")}
                    </span>
                  )}
                  {automationSnapshot?.status === "denied" && (
                    <span className="inline-flex items-center gap-1 px-2.5 py-1 rounded-full text-[11px] font-medium
                                     bg-red-500/10 text-red-600 dark:text-red-400
                                     border border-red-500/20">
                      {tAuto("settings.statusDenied")}
                    </span>
                  )}
                  {automationSnapshot?.status === "dismissed" && (
                    <span className="inline-flex items-center gap-1 px-2.5 py-1 rounded-full text-[11px] font-medium
                                     bg-gray-500/10 text-gray-500 dark:text-slate-400
                                     border border-gray-500/20">
                      {tAuto("settings.statusDismissed")}
                    </span>
                  )}
                  {(automationSnapshot?.status === "unknown" || !automationSnapshot) && (
                    <span className="inline-flex items-center gap-1 px-2.5 py-1 rounded-full text-[11px] font-medium
                                     bg-gray-500/10 text-gray-500 dark:text-slate-400
                                     border border-gray-500/20">
                      {tAuto("settings.statusUnknown")}
                    </span>
                  )}
                </div>

                {/* Action buttons — vary by status */}
                <div className="flex flex-wrap gap-2">
                  {(automationSnapshot?.status === "unknown" ||
                    automationSnapshot?.status === "dismissed") && (
                    <button
                      onClick={handleRequestAutomation}
                      className="px-3 py-1.5 text-xs font-semibold rounded-lg
                                 bg-orange-500 text-white hover:bg-orange-600
                                 transition-colors"
                    >
                      {tAuto("settings.requestButton")}
                    </button>
                  )}

                  {automationSnapshot?.status === "denied" && (
                    <>
                      <button
                        onClick={handleOpenSystemSettings}
                        className="px-3 py-1.5 text-xs font-semibold rounded-lg
                                   bg-red-500 text-white hover:bg-red-600
                                   transition-colors"
                      >
                        {tAuto("settings.openSettings")}
                      </button>
                      <button
                        onClick={handleRequestAutomation}
                        className="px-3 py-1.5 text-xs font-semibold rounded-lg
                                   border border-gray-200 dark:border-white/[0.08]
                                   text-gray-600 dark:text-gray-300
                                   bg-white/40 dark:bg-white/[0.04]
                                   hover:bg-white/70 dark:hover:bg-white/[0.08]
                                   transition-colors"
                      >
                        {tAuto("settings.reauthorizeButton")}
                      </button>
                    </>
                  )}

                  {automationSnapshot?.status === "granted" && (
                    <button
                      onClick={handleOpenSystemSettings}
                      className="px-3 py-1.5 text-xs font-semibold rounded-lg
                                 border border-gray-200 dark:border-white/[0.08]
                                 text-gray-600 dark:text-gray-300
                                 bg-white/40 dark:bg-white/[0.04]
                                 hover:bg-white/70 dark:hover:bg-white/[0.08]
                                 transition-colors"
                    >
                      {tAuto("settings.openSettings")}
                    </button>
                  )}
                </div>
              </div>
            </div>
          </div>
        </div>
      )}

        </div>
      </div>
    </div>
  );
}

{/* ===== Reusable setting row component ===== */}
function SettingRow({ label, desc, children }: { label: string; desc?: string; children: React.ReactNode }) {
  return (
    <div className="p-4 flex items-center justify-between gap-4">
      <div className="min-w-0">
        <div className="text-sm font-medium text-gray-700 dark:text-gray-300">{label}</div>
        {desc && <div className="text-xs text-gray-400 dark:text-slate-500 mt-0.5">{desc}</div>}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

function ToggleSwitch({ checked, onChange, color = "orange" }: { checked: boolean; onChange: (v: boolean) => void; color?: string }) {
  const bgColor = checked
    ? color === "amber" ? "bg-amber-500"
    : color === "green" ? "bg-green-500"
    : "bg-orange-500"
    : "bg-gray-300 dark:bg-slate-600";

  return (
    <button
      onClick={() => onChange(!checked)}
      className={`relative w-11 h-6 rounded-full transition-colors duration-200 shrink-0 ${bgColor}`}
    >
      <span className={`absolute top-0.5 left-0.5 w-5 h-5 rounded-full bg-white shadow-sm transition-transform duration-200 ${checked ? "translate-x-5" : "translate-x-0"}`} />
    </button>
  );
}

function ExportSection({ totalItems }: { totalItems: number }) {
  const { t } = useTranslation("settings");
  const { t: tc } = useTranslation("common");
  const [exportStatus, setExportStatus] = useState<"idle" | "exporting" | "done">("idle");
  const [resultMsg, setResultMsg] = useState("");
  const [rangeOpen, setRangeOpen] = useState(false);
  const [startDate, setStartDate] = useState("");
  const [endDate, setEndDate] = useState("");

  const handleExportAll = async () => {
    setExportStatus("exporting");
    try {
      await invoke("export_all_single");
      setResultMsg(t("export.exportedAll", { count: totalItems }));
      setExportStatus("done");
      setTimeout(() => setExportStatus("idle"), 3000);
    } catch (e) {
      console.error(e);
      setExportStatus("idle");
    }
  };

  const handleExportRange = async () => {
    if (!startDate || !endDate) return;
    setExportStatus("exporting");
    try {
      await invoke("export_range_single", { start: startDate, end: endDate });
      setResultMsg(t("export.exportedRange", { start: startDate, end: endDate }));
      setExportStatus("done");
      setRangeOpen(false);
      setTimeout(() => setExportStatus("idle"), 3000);
    } catch (e) {
      console.error(e);
      setExportStatus("idle");
    }
  };

  return (
    <div className="glass rounded-2xl divide-y divide-gray-100/50 dark:divide-white/[0.06]">
      {/* Export all */}
      <div className="p-4">
        <div className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">{t("export.exportAll")}</div>
        <div className="text-xs text-gray-400 dark:text-slate-500 mb-3">
          {t("export.exportAllDesc", { count: totalItems })}
        </div>
        <button
          onClick={handleExportAll}
          disabled={exportStatus === "exporting"}
          className="w-full py-2 text-sm font-medium rounded-lg border
                     text-orange-600 dark:text-orange-400 border-orange-200/50 dark:border-orange-500/20
                     bg-orange-50/50 dark:bg-orange-500/[0.06]
                     hover:bg-orange-100/50 dark:hover:bg-orange-500/[0.12]
                     disabled:opacity-50 transition-colors"
        >
          {exportStatus === "exporting" ? t("export.exporting") : t("export.exportAllBtn")}
        </button>
      </div>

      {/* Export date range */}
      <div className="p-4">
        <div className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">{t("export.exportRange")}</div>
        <div className="text-xs text-gray-400 dark:text-slate-500 mb-3">
          {t("export.exportRangeDesc")}
        </div>
        {!rangeOpen ? (
          <button
            onClick={() => {
              setRangeOpen(true);
              if (!startDate) {
                const end = new Date();
                const start = new Date();
                start.setDate(end.getDate() - 7);
                setStartDate(start.toISOString().slice(0, 10));
                setEndDate(end.toISOString().slice(0, 10));
              }
            }}
            className="w-full py-2 text-sm font-medium rounded-lg border
                       text-gray-600 dark:text-gray-300 border-gray-200/50 dark:border-white/[0.08]
                       bg-white/40 dark:bg-white/[0.04]
                       hover:bg-white/70 dark:hover:bg-white/[0.08] transition-colors"
          >
            {t("export.selectDateRange")}
          </button>
        ) : (
          <div className="space-y-2.5">
            <div>
              <div className="text-xs text-gray-500 dark:text-slate-400 mb-1">{t("export.startDate")}</div>
              <input
                type="date"
                value={startDate}
                onChange={(e) => setStartDate(e.target.value)}
                className="w-full text-sm px-3 py-1.5 rounded-lg border border-gray-200/50 dark:border-white/[0.08]
                           bg-white/50 dark:bg-white/[0.04] text-gray-800 dark:text-gray-200
                           focus:outline-none focus:ring-1 focus:ring-orange-400/50"
              />
            </div>
            <div>
              <div className="text-xs text-gray-500 dark:text-slate-400 mb-1">{t("export.endDate")}</div>
              <input
                type="date"
                value={endDate}
                onChange={(e) => setEndDate(e.target.value)}
                className="w-full text-sm px-3 py-1.5 rounded-lg border border-gray-200/50 dark:border-white/[0.08]
                           bg-white/50 dark:bg-white/[0.04] text-gray-800 dark:text-gray-200
                           focus:outline-none focus:ring-1 focus:ring-orange-400/50"
              />
            </div>
            <div className="flex gap-2">
              <button
                onClick={() => setRangeOpen(false)}
                className="flex-1 py-1.5 text-sm font-medium rounded-lg border
                           text-gray-500 dark:text-slate-400 border-gray-200/50 dark:border-white/[0.08]
                           bg-white/40 dark:bg-white/[0.04] hover:bg-white/70 dark:hover:bg-white/[0.08] transition-colors"
              >
                {tc("action.cancel")}
              </button>
              <button
                onClick={handleExportRange}
                disabled={!startDate || !endDate || exportStatus === "exporting"}
                className="flex-1 py-1.5 text-sm font-medium rounded-lg border
                           text-orange-600 dark:text-orange-400 border-orange-200/50 dark:border-orange-500/20
                           bg-orange-50/50 dark:bg-orange-500/[0.06]
                           hover:bg-orange-100/50 dark:hover:bg-orange-500/[0.12]
                           disabled:opacity-50 transition-colors"
              >
                {exportStatus === "exporting" ? t("export.exporting") : t("export.confirmExport")}
              </button>
            </div>
          </div>
        )}
      </div>

      {/* Result toast */}
      {exportStatus === "done" && (
        <div className="p-4">
          <div className="px-3 py-2 rounded-lg bg-green-500/10 dark:bg-green-500/15 border border-green-300/40 dark:border-green-500/20">
            <p className="text-xs text-green-700 dark:text-green-400 text-center">
              {resultMsg}{t("export.exportedFinderHint")}
            </p>
          </div>
        </div>
      )}

      {/* Wiki settings */}
      <WikiSettingsSection />
    </div>
  );
}

function WikiSettingsSection() {
  const { t } = useTranslation("settings");
  const { setStorageInfo } = useSettingsStore();
  const [stats, setStats] = useState<{ total_pages: number; total_edges: number; total_sources: number } | null>(null);
  const [autoCompile, setAutoCompile] = useState(true);
  const [compiling, setCompiling] = useState(false);
  const [compileResult, setCompileResult] = useState("");
  const [localRawPath, setLocalRawPath] = useState("");
  const [syncingLocalRaw, setSyncingLocalRaw] = useState(false);
  const [localRawSyncResult, setLocalRawSyncResult] = useState("");

  useEffect(() => {
    import("../../services/wikiService").then(async (ws) => {
      try {
        const s = await ws.getWikiStats();
        setStats(s);
      } catch {}
    });
    import("../../services/settingsService").then(async (ss) => {
      try {
        const settings = await ss.getSettings();
        setAutoCompile(settings.wiki_auto_compile !== "false");
        setLocalRawPath(settings.wiki_raw_source_path || "");
      } catch {}
    });
  }, []);

  const handleToggle = async () => {
    const newVal = !autoCompile;
    setAutoCompile(newVal);
    try {
      const { updateSetting } = await import("../../services/settingsService");
      await updateSetting("wiki_auto_compile", newVal ? "true" : "false");
    } catch (e) {
      console.error("Failed to update wiki setting:", e);
    }
  };

  const handleBatchCompile = async () => {
    setCompiling(true);
    setCompileResult("");
    try {
      const { triggerWikiAutoCompile } = await import("../../services/wikiService");
      const result = await triggerWikiAutoCompile();
      if (result.errors > 0) {
        setCompileResult(t("wiki.compileResultWithErrors", {
          processed: result.processed,
          compiled: result.compiled,
          errors: result.errors,
        }));
      } else {
        setCompileResult(t("wiki.compileResult", {
          processed: result.processed,
          compiled: result.compiled,
        }));
      }
      // Refresh stats
      const { getWikiStats } = await import("../../services/wikiService");
      setStats(await getWikiStats());
    } catch (e) {
      setCompileResult(t("wiki.compileFailed", { error: String(e) }));
    }
    setCompiling(false);
  };

  const handleSyncLocalRaw = async () => {
    const trimmedPath = localRawPath.trim();
    if (!trimmedPath) {
      setLocalRawSyncResult(t("wiki.localRawSyncFailed", { error: t("workspaceSetup.notConfigured") }));
      return;
    }

    setSyncingLocalRaw(true);
    setLocalRawSyncResult("");

    try {
      const [{ updateSetting }, { syncLocalRawDirectory }] = await Promise.all([
        import("../../services/settingsService"),
        import("../../services/dataHubService"),
      ]);

      await updateSetting("wiki_raw_source_path", trimmedPath);
      const result = await syncLocalRawDirectory(trimmedPath);
      const info = await invoke<{ total_items: number; disk_usage_mb: number }>("get_storage_info");
      setStorageInfo(info.total_items, info.disk_usage_mb);
      setLocalRawSyncResult(
        t("wiki.localRawSyncResult", {
          files: result.files_found,
          created: result.created,
          updated: result.updated,
          skipped: result.skipped,
          removed: result.removed,
        })
      );
    } catch (e) {
      setLocalRawSyncResult(t("wiki.localRawSyncFailed", { error: String(e) }));
    }

    setSyncingLocalRaw(false);
  };

  return (
    <div className="px-5 py-4 border-t" style={{ borderColor: "var(--color-border, #E7E5E4)" }}>
      <h3 style={{ fontSize: 14, fontWeight: 700, color: "var(--color-text-primary, #1C1917)", marginBottom: 12 }}>
        {t("wiki.title")}
      </h3>

      {/* Stats */}
      {stats && (
        <div className="flex gap-4 mb-4">
          <div className="text-center">
            <div style={{ fontSize: 18, fontWeight: 700, color: "#F97316", fontFamily: "'Cabinet Grotesk', sans-serif" }}>
              {stats.total_pages}
            </div>
            <div style={{ fontSize: 11, color: "var(--color-text-muted)" }}>{t("wiki.pages")}</div>
          </div>
          <div className="text-center">
            <div style={{ fontSize: 18, fontWeight: 700, color: "#F97316", fontFamily: "'Cabinet Grotesk', sans-serif" }}>
              {stats.total_edges}
            </div>
            <div style={{ fontSize: 11, color: "var(--color-text-muted)" }}>{t("wiki.edges")}</div>
          </div>
          <div className="text-center">
            <div style={{ fontSize: 18, fontWeight: 700, color: "#F97316", fontFamily: "'Cabinet Grotesk', sans-serif" }}>
              {stats.total_sources}
            </div>
            <div style={{ fontSize: 11, color: "var(--color-text-muted)" }}>{t("wiki.sources")}</div>
          </div>
        </div>
      )}

      {/* Auto compile toggle */}
      <div className="flex items-center justify-between py-2">
        <div>
          <p style={{ fontSize: 13, fontWeight: 500, color: "var(--color-text-primary)" }}>{t("wiki.autoCompile")}</p>
          <p style={{ fontSize: 11, color: "var(--color-text-muted)" }}>{t("wiki.autoCompileDesc")}</p>
        </div>
        <button
          onClick={handleToggle}
          className="relative w-10 h-5 rounded-full transition-colors"
          style={{ backgroundColor: autoCompile ? "#F97316" : "var(--color-border, #E7E5E4)" }}
        >
          <div
            className="absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform"
            style={{ left: autoCompile ? 22 : 2 }}
          />
        </button>
      </div>

      {/* Batch compile button */}
      <div className="mt-3">
        <button
          onClick={handleBatchCompile}
          disabled={compiling}
          className="px-4 py-2 rounded-lg text-xs font-medium transition-all disabled:opacity-40"
          style={{
            backgroundColor: "#F9731615",
            color: "#F97316",
            border: "1px solid #F9731630",
          }}
        >
          {compiling ? t("wiki.compiling") : t("wiki.batchCompile")}
        </button>
        {compileResult && (
          <p className="mt-2" style={{ fontSize: 11, color: "var(--color-text-secondary)" }}>
            {compileResult}
          </p>
        )}
      </div>

      <div className="mt-5 pt-4 border-t" style={{ borderColor: "var(--color-border, #E7E5E4)" }}>
        <div style={{ fontSize: 13, fontWeight: 500, color: "var(--color-text-primary)" }}>{t("wiki.localRawPath")}</div>
        <p className="mt-1" style={{ fontSize: 11, color: "var(--color-text-muted)" }}>
          {t("wiki.localRawPathDesc")}
        </p>
        <div className="mt-3 flex gap-2">
          <input
            type="text"
            value={localRawPath}
            onChange={(e) => setLocalRawPath(e.target.value)}
            placeholder="/Users/you/knowledge/raw"
            className="flex-1 text-sm px-3 py-2 rounded-lg border border-gray-200/50 dark:border-white/[0.08]
                       bg-white/50 dark:bg-white/[0.04] text-gray-800 dark:text-gray-200
                       focus:outline-none focus:ring-1 focus:ring-orange-400/50"
          />
          <button
            onClick={handleSyncLocalRaw}
            disabled={syncingLocalRaw}
            className="px-4 py-2 rounded-lg text-xs font-medium transition-all disabled:opacity-40"
            style={{
              backgroundColor: "#F9731615",
              color: "#F97316",
              border: "1px solid #F9731630",
            }}
          >
            {syncingLocalRaw ? t("wiki.localRawSyncing") : t("wiki.localRawSync")}
          </button>
        </div>
        {localRawSyncResult && (
          <p className="mt-2" style={{ fontSize: 11, color: "var(--color-text-secondary)" }}>
            {localRawSyncResult}
          </p>
        )}
      </div>
    </div>
  );
}
