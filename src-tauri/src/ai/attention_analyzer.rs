use crate::storage::models::ContentForAnalysis;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::Semaphore;

/// Local providers (Ollama, Custom) typically serve one request at a time.
/// Serialize all requests to them to avoid starving/timing out concurrent calls.
fn local_provider_gate() -> &'static Semaphore {
    static GATE: OnceLock<Semaphore> = OnceLock::new();
    GATE.get_or_init(|| Semaphore::new(1))
}

// ====================================================================
// Data Types (v2: Briefing — kept for backwards compat)
// ====================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingTopic {
    pub id: String,
    pub rank: u32,
    pub insight_title: String,
    pub deep_analysis: String,
    pub key_findings: Vec<String>,
    pub suggestion: Option<String>,
    pub evidence_indices: Vec<usize>,
    pub content_count: u32,
    pub span_days: u32,
    pub trend: String,
    pub tag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingMeta {
    pub total_content: u32,
    pub window_days: u32,
    pub analysis_depth: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingAnalysis {
    pub format_version: u32,
    pub topics: Vec<BriefingTopic>,
    pub meta: BriefingMeta,
}

// ====================================================================
// Data Types (v3: RadarReport — 7-section scrolling report)
// ====================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadarReport {
    pub meta: RadarMeta,
    pub at_a_glance: Vec<Glance>,
    pub info_diet: InfoDiet,
    pub subconscious: Vec<SubconsciousItem>,
    pub graveyard: Graveyard,
    pub blind_spots: Vec<BlindSpot>,
    pub actions: Vec<Action>,
    pub heatmap: Vec<HeatmapDay>,
    pub topic_cloud: Vec<TopicItem>,
    pub verdict: Verdict,
    pub footer: Footer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadarMeta {
    pub date_range: String,
    pub total_items: u32,
    pub active_days: u32,
    pub annotated_items: u32,
    pub annotation_rate: String,
    pub source_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Glance {
    pub text: String,
    pub highlight: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoDiet {
    pub sources: Vec<DietSource>,
    pub depth_ratio: DepthRatio,
    pub dominant_topic: DominantTopic,
    #[serde(default)]
    pub language_ratio: Option<LanguageRatio>,
    pub alert: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DietSource {
    pub name: String,
    pub count: u32,
    pub percent: f64,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthRatio {
    pub deep: f64,
    pub shallow: f64,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DominantTopic {
    pub name: String,
    pub percent: f64,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageRatio {
    pub chinese: f64,
    pub english: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubconsciousItem {
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub evidence_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Graveyard {
    #[serde(default)]
    pub forgotten_count: Option<u32>,
    #[serde(default)]
    pub forgotten_percent: Option<f64>,
    pub alert: String,
    pub top_picks: Vec<GraveyardPick>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraveyardPick {
    pub rank: u32,
    pub title: String,
    pub reason: String,
    pub tags: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlindSpot {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub icon: String,
    pub title: String,
    pub desc: String,
    #[serde(rename = "ref")]
    pub action_ref: String,
    pub time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeatmapDay {
    pub date: String,
    pub count: u32,
    pub is_peak: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicItem {
    pub name: String,
    pub percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    pub text: String,
    pub highlights: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Footer {
    pub date_range: String,
    pub total: u32,
    pub active_days: u32,
    pub total_days: u32,
}

// ====================================================================
// Prompt Builder (v2: Briefing — kept for backwards compat)
// ====================================================================

/// Build system prompt and user message from content items (old v2 format).
/// Each item is (id, raw_text, source_url, captured_at).
pub fn build_prompt(
    items: &[(String, Option<String>, Option<String>, String)],
) -> (String, String) {
    let count = items.len();
    let max_chars: usize = 3000;

    let system_prompt =
        r#"你是用户的私人知识分析师。你的核心任务是：找到用户自己没注意到的联系和规律。

不要给出用户已经知道的信息（如"你关注了 AI"），而是找到令人惊讶的发现。

具体做法：
1. 找出用户最集中关注的 1-3 个方向（最多 3 个，如果没有明确方向就返回空的 topics 数组）
2. 对每个方向，找到跨内容的**意外发现**：
   - 两篇看似不相关的内容之间的隐藏联系
   - 用户无意识的行为模式（比如"你一直在围绕某个决定收集信息"）
   - 内容之间的矛盾或有趣对比
   - 不要复述每篇文章的内容，找到贯穿多篇的意外规律
3. 对排名第 1 的方向，给出一个具体可行动的建议（suggestion 字段），其他方向 suggestion 设为 null
4. 生成洞察性标题（不是主题名"AI 产品设计"，而是一个让人想点进去看的发现）

重要规则：
- 使用内容的**序号**（从 0 开始的 index）来引用内容，放在 evidence_indices 数组中
- topics 数组最多 3 个元素，按重要性排序（rank 1 最重要）
- 如果内容太分散没有明显方向，返回空的 topics 数组，这完全正常
- tag 只能是以下值之一："核心关注"、"次要关注"、"新兴关注"、"背景关注"
- trend 只能是以下值之一："growing"、"emerging"、"stable"、"fading"

请严格按以下 JSON 格式返回：
{
  "format_version": 2,
  "topics": [
    {
      "id": "topic_1",
      "rank": 1,
      "insight_title": "你保存的 3 篇文章指向了同一个你还没做的决定",
      "deep_analysis": "详细的跨内容分析段落...",
      "key_findings": ["发现1", "发现2", "发现3"],
      "suggestion": "具体可行动的建议...",
      "evidence_indices": [0, 3, 5, 7],
      "content_count": 12,
      "span_days": 9,
      "trend": "growing",
      "tag": "核心关注"
    }
  ],
  "meta": {
    "total_content": 42,
    "window_days": 14,
    "analysis_depth": "deep"
  }
}"#
        .to_string();

    let mut content_lines = Vec::with_capacity(count);
    for (i, (_id, raw_text, source_url, captured_at)) in items.iter().enumerate() {
        let text = raw_text.as_deref().unwrap_or("[无文本]");
        let truncated = truncate_str(text, max_chars);
        let url_part = source_url
            .as_deref()
            .map(|u| format!(" | 来源: {}", u))
            .unwrap_or_default();
        content_lines.push(format!(
            "[{}] (时间={}{}) {}",
            i, captured_at, url_part, truncated
        ));
    }

    let user_message = format!(
        "以下是用户最近 14 天收集的 {} 条内容，请深入分析并提炼洞察：\n\n{}",
        count,
        content_lines.join("\n\n")
    );

    (system_prompt, user_message)
}

// ====================================================================
// Prompt Builder (v3: Radar Report)
// ====================================================================

/// Build system prompt and user message for the v3 radar report.
pub fn build_prompt_v2(
    items: &[ContentForAnalysis],
    stats: &serde_json::Value,
) -> (String, String) {
    let system_prompt = r#"你是 OpenWiki 洞察，专门分析用户信息收藏行为的 AI 分析师。

你会收到两部分数据：
1. stats：用户这段时间的统计摘要（来源分布、时段分布、标注率等）
2. items：每条保存记录的基本信息

你的任务：基于这些数据，生成一份深度行为分析报告。

## 分析原则
- 用"你"称呼用户，直接说话，不要客套
- 有观点、敢判断，不说"可能是A也可能是B"，直接说"是A，因为数据显示X"
- 每个结论必须引用具体数字或内容（"你有17条关于XX的保存"，不是"你很关注XX"）
- 发现用户没意识到的模式，比描述显而易见的事实更有价值
- 区分"信息焦虑"和"真正的学习意图"，诚实指出
- 不要只说好的，摩擦点和问题要说清楚

## 输出格式
严格输出以下 JSON，第一个字符必须是 {，最后一个字符必须是 }，不输出任何其他内容：

{
  "meta": {
    "date_range": "字符串",
    "total_items": 数字,
    "active_days": 数字,
    "annotated_items": 数字,
    "annotation_rate": "如'29%'",
    "source_count": 数字
  },
  "at_a_glance": [
    {
      "text": "洞察段落，150字以内，必须包含具体数字",
      "highlight": "段落中最关键的短语，10字以内"
    }
  ],
  "info_diet": {
    "sources": [
      {"name": "来源名", "count": 数字, "percent": 数字, "color": "wechat|chrome|xiaoyun|other"}
    ],
    "depth_ratio": {"deep": 百分比数字, "shallow": 百分比数字, "label": "深度长文 X% / 碎片 Y%"},
    "dominant_topic": {"name": "最多主题", "percent": 数字, "label": "如'重度偏食'"},
    "alert": "饮食结构警告，1句"
  },
  "subconscious": [
    {
      "title": "发现X：标题，20字以内",
      "body": "详细解释，100字以内，必须引用具体条数或内容",
      "evidence_count": 数字
    }
  ],
  "graveyard": {
    "alert": "风险提示，1句",
    "top_picks": [
      {
        "rank": 数字,
        "title": "内容标题",
        "reason": "为什么值得重读，80字以内，说清楚用什么问题去读",
        "tags": ["标签"]
      }
    ]
  },
  "blind_spots": [
    {
      "title": "盲点X：标题，20字以内",
      "body": "解释，80字以内，说清楚缺失了什么以及为什么重要"
    }
  ],
  "actions": [
    {
      "icon": "单个emoji",
      "title": "行动标题，25字以内",
      "desc": "具体怎么做，60字以内，必须绑定到具体保存内容",
      "ref": "关联内容名称",
      "time": "预计时间如'90分钟'"
    }
  ],
  "heatmap": [
    {"date": "MM/DD", "count": 数字, "is_peak": true或false}
  ],
  "topic_cloud": [
    {"name": "主题名", "percent": 数字}
  ],
  "verdict": {
    "text": "一句话总结，50字以内，辛辣有力，点出核心矛盾",
    "highlights": ["需要高亮的关键词1", "关键词2", "关键词3"]
  },
  "footer": {
    "date_range": "如'03-21~04-05'",
    "total": 数字,
    "active_days": 数字,
    "total_days": 数字
  }
}

## 各板块生成规则

### at_a_glance（最后生成，汇总其他板块核心结论）
- 2-3条，每条聚焦一个核心洞察
- 第一条：你真正在追的是什么（不是你以为的）
- 第二条：行为模式的核心矛盾
- 第三条（可选）：最意外的发现

### subconscious（最有价值的板块）
- 3-4条，每条都是用户"没意识到的"关注
- 每条必须有具体证据

### graveyard
- top_picks 选择深度内容中最值得重读的 3 条
- reason 要说清楚"带着什么问题去读"

### blind_spots
- 从主题分布中找"高频主题的对立面"

### actions
- 每条必须绑定到具体的保存内容
- 3条，不多不少"#
        .to_string();

    let max_chars: usize = 3000;
    let mut item_jsons = Vec::with_capacity(items.len());
    for item in items {
        let text = item.raw_text.as_deref().unwrap_or("");
        let excerpt = truncate_str(text, max_chars);

        let mut obj = serde_json::json!({
            "date": item.captured_at.get(..10).unwrap_or(""),
            "time": item.captured_at.get(11..16).unwrap_or(""),
            "source": &item.source_app,
            "type": &item.content_type,
            "content_excerpt": excerpt,
        });

        if let Some(ref s) = item.summary {
            if !s.is_empty() {
                obj["summary"] = serde_json::json!(s);
            }
        }
        if let Some(ref t) = item.tags {
            if !t.is_empty() {
                obj["tags"] = serde_json::json!(t);
            }
        }
        if let Some(ref n) = item.user_note {
            if !n.is_empty() {
                obj["user_note"] = serde_json::json!(n);
            }
        }

        item_jsons.push(obj);
    }

    let user_data = serde_json::json!({
        "stats": stats,
        "items": item_jsons,
    });

    let user_message = serde_json::to_string(&user_data).unwrap_or_default();

    (system_prompt, user_message)
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = chars[..max_chars].iter().collect();
        format!("{}...", truncated)
    }
}

// ====================================================================
// JSON Validators
// ====================================================================

/// Parse and validate a BriefingAnalysis JSON string (v2).
pub fn validate_analysis(json_str: &str, item_count: usize) -> Result<BriefingAnalysis, String> {
    let cleaned = extract_json(json_str);

    let mut analysis: BriefingAnalysis =
        serde_json::from_str(&cleaned).map_err(|e| format!("JSON parse failed: {}", e))?;

    analysis.topics.truncate(3);

    for topic in &mut analysis.topics {
        topic.evidence_indices.retain(|&idx| idx < item_count);
    }

    Ok(analysis)
}

/// Parse and validate a RadarReport JSON string (v3).
pub fn validate_radar_report(json_str: &str) -> Result<RadarReport, String> {
    let cleaned = extract_json(json_str);

    let report: RadarReport =
        serde_json::from_str(&cleaned).map_err(|e| format!("RadarReport JSON parse failed: {}", e))?;

    let required_lists = [
        ("at_a_glance", report.at_a_glance.len()),
        ("info_diet.sources", report.info_diet.sources.len()),
        ("subconscious", report.subconscious.len()),
        ("graveyard.top_picks", report.graveyard.top_picks.len()),
        ("blind_spots", report.blind_spots.len()),
        ("actions", report.actions.len()),
        ("heatmap", report.heatmap.len()),
        ("topic_cloud", report.topic_cloud.len()),
    ];

    for (name, len) in required_lists {
        if len == 0 {
            return Err(format!("{} cannot be empty", name));
        }
    }

    if report.verdict.text.trim().is_empty() {
        return Err("verdict.text cannot be empty".to_string());
    }

    if report.footer.total_days == 0 {
        return Err("footer.total_days must be greater than 0".to_string());
    }

    if report
        .actions
        .iter()
        .any(|action| action.action_ref.trim().is_empty())
    {
        return Err("actions.ref cannot be empty".to_string());
    }

    Ok(report)
}

fn extract_json(s: &str) -> String {
    let trimmed = s.trim();
    if let Some(start) = trimmed.find("```json") {
        let after_marker = &trimmed[start + 7..];
        if let Some(end) = after_marker.find("```") {
            return after_marker[..end].trim().to_string();
        }
    }
    if let Some(start) = trimmed.find("```") {
        let after_marker = &trimmed[start + 3..];
        if let Some(end) = after_marker.find("```") {
            return after_marker[..end].trim().to_string();
        }
    }
    trimmed.to_string()
}

// ====================================================================
// API Caller
// ====================================================================

#[derive(Debug, Clone)]
pub enum AnalysisProvider {
    Anthropic,
    OpenAi,
    OpenRouter,
    DashScope,
    MiniMax,
    DeepSeek,
    Custom { base_url: String },
    Ollama { base_url: String },
    LmStudio { base_url: String },
}

impl AnalysisProvider {
    pub fn from_str(s: &str) -> Self {
        Self::from_str_with_base(s, "")
    }

    pub fn from_str_with_base(s: &str, base_url: &str) -> Self {
        match s.to_lowercase().as_str() {
            "openai" => AnalysisProvider::OpenAi,
            "openrouter" => AnalysisProvider::OpenRouter,
            "dashscope" => AnalysisProvider::DashScope,
            "minimax" => AnalysisProvider::MiniMax,
            "deepseek" => AnalysisProvider::DeepSeek,
            "google" => AnalysisProvider::OpenAi, // Google only uses OAuth, not API keys; this is a fallback guard
            "custom" => AnalysisProvider::Custom { base_url: base_url.to_string() },
            "ollama" => AnalysisProvider::Ollama {
                base_url: if base_url.is_empty() {
                    "http://localhost:11434/v1".to_string()
                } else {
                    base_url.to_string()
                },
            },
            "lmstudio" => AnalysisProvider::LmStudio {
                base_url: if base_url.is_empty() {
                    "http://localhost:1234/v1".to_string()
                } else {
                    base_url.to_string()
                },
            },
            _ => AnalysisProvider::Anthropic,
        }
    }
}

// --- Anthropic types ---

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<ApiMessage>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContentBlock>,
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlock {
    text: String,
}

// --- OpenAI-compatible types ---

#[derive(Debug, Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<ApiMessage>,
    max_tokens: u32,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enable_thinking: Option<bool>,
    // Ollama-specific: disables <think> reasoning traces on qwen3-family models.
    // Without this, Ollama's OpenAI-compatible endpoint lets qwen3.5 churn on
    // a hidden reasoning chain for minutes even on trivial prompts.
    #[serde(skip_serializing_if = "Option::is_none")]
    think: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    format_type: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: ApiMessage,
}

/// Call the AI API directly to perform attention analysis.
/// Returns the raw response text.
/// `expect_json` hints the provider that the response must be valid JSON —
/// Ollama uses this to enable grammar-constrained decoding via `format: "json"`,
/// which weaker local models (e.g. qwen3.5:9b) need to avoid emitting broken JSON.
pub async fn call_analysis_api(
    provider: &AnalysisProvider,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    user_message: &str,
    max_tokens: u32,
    expect_json: bool,
) -> Result<String, String> {
    // Local models (Ollama/custom) need much longer timeouts — cold-starts and
    // slower inference on consumer hardware can push a single request past a minute.
    let is_local_provider = matches!(
        provider,
        AnalysisProvider::Custom { .. }
            | AnalysisProvider::Ollama { .. }
            | AnalysisProvider::LmStudio { .. }
    );
    let timeout_secs = if is_local_provider { 900 } else { 120 };
    let http_client = Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    // Serialize concurrent calls against local providers so we don't starve
    // one another (Ollama by default only serves one request at a time).
    let _permit = if is_local_provider {
        Some(
            local_provider_gate()
                .acquire()
                .await
                .map_err(|e| format!("Local provider gate closed: {}", e))?,
        )
    } else {
        None
    };

    match provider {
        AnalysisProvider::Ollama { base_url } => {
            // Ollama's OpenAI-compatible /v1/chat/completions endpoint silently
            // ignores the `think` parameter, so thinking models like qwen3.x burn
            // the entire token budget on hidden reasoning and return empty content.
            // The native /api/chat endpoint honors `think: false`, so we use it.
            let host = {
                let s = if base_url.is_empty() {
                    "http://localhost:11434"
                } else {
                    base_url.as_str()
                };
                let mut h = s.trim_end_matches('/').to_string();
                for suffix in ["/chat/completions", "/v1"] {
                    if h.ends_with(suffix) {
                        h.truncate(h.len() - suffix.len());
                        h = h.trim_end_matches('/').to_string();
                    }
                }
                h
            };
            let url = format!("{}/api/chat", host);

            let mut messages = Vec::<serde_json::Value>::new();
            if !system_prompt.is_empty() {
                messages.push(serde_json::json!({
                    "role": "system",
                    "content": system_prompt,
                }));
            }
            messages.push(serde_json::json!({
                "role": "user",
                "content": user_message,
            }));

            let effective_max = max_tokens.max(8192);
            let mut body = serde_json::json!({
                "model": model,
                "messages": messages,
                "stream": false,
                "think": false,
                "options": {
                    "num_predict": effective_max as i64,
                    "temperature": 0.5,
                },
            });
            if expect_json {
                body["format"] = serde_json::Value::String("json".to_string());
            }

            let resp = http_client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Ollama API request failed: {}", e))?;

            let status = resp.status();
            let text = resp
                .text()
                .await
                .map_err(|e| format!("Failed to read Ollama response: {}", e))?;

            if !status.is_success() {
                return Err(format!("Ollama API error ({}): {}", status, text));
            }

            let v: serde_json::Value = serde_json::from_str(&text)
                .map_err(|e| format!("Failed to parse Ollama response: {} body: {}", e, text))?;
            let content = v
                .pointer("/message/content")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();

            if content.is_empty() {
                log::warn!(
                    "Ollama returned empty content — model={} done_reason={:?} raw_body={}",
                    model,
                    v.get("done_reason").and_then(|r| r.as_str()),
                    text.chars().take(500).collect::<String>()
                );
            }

            Ok(content)
        }
        AnalysisProvider::Anthropic => {
            let body = AnthropicRequest {
                model: model.to_string(),
                max_tokens,
                system: system_prompt.to_string(),
                messages: vec![ApiMessage {
                    role: "user".to_string(),
                    content: user_message.to_string(),
                }],
            };

            let resp = http_client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Anthropic API request failed: {}", e))?;

            let status = resp.status();
            let text = resp
                .text()
                .await
                .map_err(|e| format!("Failed to read Anthropic response: {}", e))?;

            if !status.is_success() {
                return Err(format!("Anthropic API error ({}): {}", status, text));
            }

            let parsed: AnthropicResponse = serde_json::from_str(&text)
                .map_err(|e| format!("Failed to parse Anthropic response: {}", e))?;

            Ok(parsed
                .content
                .first()
                .map(|c| c.text.clone())
                .unwrap_or_default())
        }
        AnalysisProvider::OpenAi | AnalysisProvider::OpenRouter | AnalysisProvider::DashScope | AnalysisProvider::MiniMax | AnalysisProvider::DeepSeek | AnalysisProvider::Custom { .. } | AnalysisProvider::LmStudio { .. } => {
            let url_owned: String;
            let url: &str = match provider {
                AnalysisProvider::OpenRouter => "https://openrouter.ai/api/v1/chat/completions",
                AnalysisProvider::DashScope => {
                    "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions"
                }
                AnalysisProvider::MiniMax => "https://api.minimax.io/v1/chat/completions",
                AnalysisProvider::DeepSeek => "https://api.deepseek.com/v1/chat/completions",
                AnalysisProvider::Custom { base_url } | AnalysisProvider::LmStudio { base_url } => {
                    if base_url.is_empty() {
                        return Err("Custom provider requires a base URL".to_string());
                    }
                    let trimmed = base_url.trim_end_matches('/');
                    url_owned = if trimmed.ends_with("/chat/completions") {
                        trimmed.to_string()
                    } else {
                        format!("{}/chat/completions", trimmed)
                    };
                    url_owned.as_str()
                }
                _ => "https://api.openai.com/v1/chat/completions",
            };

            let is_local = matches!(
                provider,
                AnalysisProvider::Custom { .. } | AnalysisProvider::LmStudio { .. }
            );

            let mut messages = Vec::new();
            if !system_prompt.is_empty() {
                messages.push(ApiMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                });
            }
            messages.push(ApiMessage {
                role: "user".to_string(),
                content: user_message.to_string(),
            });

            // Thinking models (qwen3, qwen3.5, deepseek-r1) burn large chunks of the
            // token budget on their internal reasoning trace before emitting the real
            // answer. Give local providers a much bigger ceiling so the final output
            // doesn't get truncated.
            let effective_max_tokens = if is_local {
                max_tokens.max(8192)
            } else {
                max_tokens
            };

            // Force `response_format: json_object` whenever we can.
            // Without it, models — especially those served via commercial
            // relays — happily reply with plain conversational prose
            // ("根据你的反馈...") that breaks downstream JSON parsing.
            //
            // Local providers (Ollama, LM Studio) are the exception:
            // their OpenAI-compatible shims sometimes deadlock or 400
            // on this flag, particularly with thinking-model variants.
            // We keep them on the markdown-aware parser fallback.
            let response_format = match provider {
                AnalysisProvider::LmStudio { .. } => None,
                _ => Some(ResponseFormat {
                    format_type: "json_object".to_string(),
                }),
            };

            let enable_thinking = match provider {
                AnalysisProvider::DashScope => Some(false),
                _ => None,
            };

            let body = OpenAiRequest {
                model: model.to_string(),
                messages,
                max_tokens: effective_max_tokens,
                temperature: 0.5,
                response_format,
                enable_thinking,
                think: if is_local { Some(false) } else { None },
                stream: None,
            };

            let mut req = http_client
                .post(url)
                .header("Authorization", format!("Bearer {}", api_key))
                .header("Content-Type", "application/json");

            if matches!(provider, AnalysisProvider::OpenRouter) {
                req = req
                    .header("HTTP-Referer", "https://openwiki.app")
                    .header("X-Title", "Xiaoyun");
            }

            let resp = req
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("API request failed: {}", e))?;

            let status = resp.status();
            let text = resp
                .text()
                .await
                .map_err(|e| format!("Failed to read API response: {}", e))?;

            if !status.is_success() {
                return Err(format!("API error ({}): {}", status, text));
            }

            let parsed: OpenAiResponse =
                serde_json::from_str(&text).map_err(|e| format!("Failed to parse API response: {}", e))?;

            Ok(parsed
                .choices
                .first()
                .map(|c| c.message.content.clone())
                .unwrap_or_default())
        }
    }
}

/// Call DashScope with SSE streaming + thinking mode enabled.
/// Accumulates reasoning_content and content from delta chunks.
/// Returns the final content (not reasoning).
pub async fn call_dashscope_streaming(
    api_key: &str,
    model: &str,
    system_prompt: &str,
    user_message: &str,
    max_tokens: u32,
) -> Result<String, String> {
    let http_client = Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let url = "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions";

    let mut messages = Vec::new();
    if !system_prompt.is_empty() {
        messages.push(serde_json::json!({"role": "system", "content": system_prompt}));
    }
    messages.push(serde_json::json!({"role": "user", "content": user_message}));

    let body = serde_json::json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
        "temperature": 0.7,
        "enable_thinking": true,
        "stream": true,
    });

    let mut resp = http_client
        .post(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("DashScope SSE request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("DashScope API error ({}): {}", status, text));
    }

    // Parse SSE stream manually using chunk() (no stream feature needed)
    let mut buffer = String::new();
    let mut content_acc = String::new();
    let mut reasoning_acc = String::new();
    let mut pending_data_lines: Vec<String> = Vec::new();

    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("Failed to read SSE stream: {}", e))?
    {
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].trim_end_matches('\r').to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if line.is_empty() {
                if process_dashscope_sse_event(
                    &mut pending_data_lines,
                    &mut content_acc,
                    &mut reasoning_acc,
                )? {
                    return Ok(content_acc);
                }
                continue;
            }

            if line.starts_with("event:") || line.starts_with("id:") {
                continue;
            }

            if let Some(data) = line.strip_prefix("data:") {
                pending_data_lines.push(data.trim().to_string());
            }
        }
    }

    if !buffer.trim().is_empty() {
        let line = buffer.trim_end_matches('\r');
        if let Some(data) = line.strip_prefix("data:") {
            pending_data_lines.push(data.trim().to_string());
        }
    }

    if process_dashscope_sse_event(
        &mut pending_data_lines,
        &mut content_acc,
        &mut reasoning_acc,
    )? {
        return Ok(content_acc);
    }

    if content_acc.is_empty() {
        Err("DashScope SSE stream ended but no content received".to_string())
    } else {
        Ok(content_acc)
    }
}

// ====================================================================
// Codex OAuth Helper
// ====================================================================

/// Try calling via Codex OAuth if available.
/// Returns `Some(Ok(text))` on success, `Some(Err(msg))` on Codex error,
/// or `None` if the user is not logged in via OAuth (fall back to API key).
/// `is_deep` = true for radar/insight reports, false for summary/tags.
/// When model is "auto", picks the right model automatically:
///   GPT summary: gpt-5.4-mini | GPT radar: gpt-5.4, fallback gpt-5.3-codex
pub async fn try_codex_call(
    db: std::sync::Arc<crate::storage::database::Database>,
    system_prompt: &str,
    user_message: &str,
    temperature: f32,
    is_deep: bool,
) -> Option<Result<String, String>> {
    let (access_token, account_id) = crate::ai::oauth::get_valid_token(db.clone()).await?;
    let repo = crate::storage::repository::Repository::new(db);
    let saved_model = repo
        .get_setting("ai_model")
        .ok()
        .flatten()
        .unwrap_or_else(|| "auto".to_string());

    let model = if saved_model == "auto" {
        if is_deep { "gpt-5.4" } else { "gpt-5.4-mini" }
    } else {
        &saved_model
    };

    let result = crate::ai::codex_api::call_codex_api(
        &access_token, &account_id, model, system_prompt, user_message, temperature,
    ).await;

    // Auto fallback for deep tasks: gpt-5.4 → gpt-5.3-codex
    if saved_model == "auto" && is_deep && result.is_err() {
        log::warn!("Auto: {} failed, falling back to gpt-5.3-codex", model);
        return Some(crate::ai::codex_api::call_codex_api(
            &access_token, &account_id, "gpt-5.3-codex", system_prompt, user_message, temperature,
        ).await);
    }

    Some(result)
}

/// When model is "auto", picks the right model automatically:
///   Gemini summary: gemini-3-flash | Gemini radar: claude-opus-4-6-thinking, fallback gemini-3.1-pro-high
pub async fn try_gemini_call(
    db: std::sync::Arc<crate::storage::database::Database>,
    system_prompt: &str,
    user_message: &str,
    temperature: f32,
    is_deep: bool,
) -> Option<Result<String, String>> {
    let (access_token, project_id) = crate::ai::gemini_oauth::get_valid_token(db.clone()).await?;
    let repo = crate::storage::repository::Repository::new(db);
    let saved_model = repo.get_setting("ai_model").ok().flatten()
        .unwrap_or_else(|| "auto".to_string());

    let model = if saved_model == "auto" {
        if is_deep { "claude-opus-4-6-thinking" } else { "gemini-3-flash" }
    } else {
        &saved_model
    };

    let result = crate::ai::gemini_api::call_gemini_api(
        &access_token, &project_id, model, system_prompt, user_message, temperature,
    ).await;

    // Auto fallback for deep tasks: claude-opus-4-6-thinking → gemini-3.1-pro-high
    if saved_model == "auto" && is_deep && result.is_err() {
        log::warn!("Auto: {} failed, falling back to gemini-3.1-pro-high", model);
        return Some(crate::ai::gemini_api::call_gemini_api(
            &access_token, &project_id, "gemini-3.1-pro-high", system_prompt, user_message, temperature,
        ).await);
    }

    Some(result)
}

fn process_dashscope_sse_event(
    pending_data_lines: &mut Vec<String>,
    content_acc: &mut String,
    reasoning_acc: &mut String,
) -> Result<bool, String> {
    if pending_data_lines.is_empty() {
        return Ok(false);
    }

    let payload = pending_data_lines.join("\n");
    pending_data_lines.clear();
    let trimmed_payload = payload.trim();

    if trimmed_payload == "[DONE]" {
        return Ok(true);
    }

    let parsed: serde_json::Value = serde_json::from_str(trimmed_payload)
        .map_err(|e| format!("DashScope SSE JSON parse failed: {}", e))?;

    if let Some(error) = parsed.get("error") {
        return Err(format!("DashScope SSE error: {}", error));
    }

    if let Some(delta) = parsed
        .get("choices")
        .and_then(|choices| choices.as_array())
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("delta"))
    {
        if let Some(reasoning) = delta
            .get("reasoning_content")
            .and_then(|reasoning| reasoning.as_str())
        {
            reasoning_acc.push_str(reasoning);
        }

        if let Some(content) = delta.get("content").and_then(|content| content.as_str()) {
            content_acc.push_str(content);
        }
    }

    Ok(false)
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_valid_briefing_json(evidence_indices: &[usize]) -> String {
        let indices_str = evidence_indices
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            r#"{{
  "format_version": 2,
  "topics": [
    {{
      "id": "topic_1",
      "rank": 1,
      "insight_title": "测试洞察标题",
      "deep_analysis": "测试深度分析",
      "key_findings": ["发现1", "发现2"],
      "suggestion": "测试建议",
      "evidence_indices": [{}],
      "content_count": 5,
      "span_days": 7,
      "trend": "growing",
      "tag": "核心关注"
    }}
  ],
  "meta": {{
    "total_content": 10,
    "window_days": 14,
    "analysis_depth": "deep"
  }}
}}"#,
            indices_str
        )
    }

    #[test]
    fn test_validate_analysis_valid() {
        let json = make_valid_briefing_json(&[0, 1, 2, 3]);
        let result = validate_analysis(&json, 5);
        assert!(result.is_ok());
        let analysis = result.unwrap();
        assert_eq!(analysis.format_version, 2);
        assert_eq!(analysis.topics.len(), 1);
        assert_eq!(analysis.topics[0].evidence_indices.len(), 4);
    }

    #[test]
    fn test_validate_analysis_out_of_bounds() {
        let json = make_valid_briefing_json(&[0, 1, 5, 10]);
        let result = validate_analysis(&json, 3);
        assert!(result.is_ok());
        let analysis = result.unwrap();
        assert_eq!(analysis.topics[0].evidence_indices.len(), 2);
        assert_eq!(analysis.topics[0].evidence_indices, vec![0, 1]);
    }

    #[test]
    fn test_validate_analysis_all_out_of_bounds() {
        let json = make_valid_briefing_json(&[5, 6, 7]);
        let result = validate_analysis(&json, 3);
        assert!(result.is_ok());
        let analysis = result.unwrap();
        assert!(analysis.topics[0].evidence_indices.is_empty());
    }

    #[test]
    fn test_validate_analysis_invalid_json() {
        let result = validate_analysis("not json at all", 5);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("JSON parse failed"));
    }

    #[test]
    fn test_validate_analysis_empty_topics() {
        let json = r#"{
            "format_version": 2,
            "topics": [],
            "meta": { "total_content": 10, "window_days": 14, "analysis_depth": "deep" }
        }"#;
        let result = validate_analysis(json, 5);
        assert!(result.is_ok());
        let analysis = result.unwrap();
        assert!(analysis.topics.is_empty());
    }

    #[test]
    fn test_validate_analysis_caps_at_3_topics() {
        let json = r#"{
            "format_version": 2,
            "topics": [
                { "id": "t1", "rank": 1, "insight_title": "A", "deep_analysis": "", "key_findings": [], "suggestion": null, "evidence_indices": [0], "content_count": 1, "span_days": 1, "trend": "stable", "tag": "核心关注" },
                { "id": "t2", "rank": 2, "insight_title": "B", "deep_analysis": "", "key_findings": [], "suggestion": null, "evidence_indices": [1], "content_count": 1, "span_days": 1, "trend": "stable", "tag": "次要关注" },
                { "id": "t3", "rank": 3, "insight_title": "C", "deep_analysis": "", "key_findings": [], "suggestion": null, "evidence_indices": [2], "content_count": 1, "span_days": 1, "trend": "stable", "tag": "新兴关注" },
                { "id": "t4", "rank": 4, "insight_title": "D", "deep_analysis": "", "key_findings": [], "suggestion": null, "evidence_indices": [3], "content_count": 1, "span_days": 1, "trend": "stable", "tag": "背景关注" }
            ],
            "meta": { "total_content": 10, "window_days": 14, "analysis_depth": "deep" }
        }"#;
        let result = validate_analysis(json, 10);
        assert!(result.is_ok());
        let analysis = result.unwrap();
        assert_eq!(analysis.topics.len(), 3);
    }

    #[test]
    fn test_validate_analysis_markdown_wrapped() {
        let json = format!("```json\n{}\n```", make_valid_briefing_json(&[0, 1]));
        let result = validate_analysis(&json, 5);
        assert!(result.is_ok());
    }

    #[test]
    fn test_build_prompt_truncation() {
        let items: Vec<(String, Option<String>, Option<String>, String)> = (0..5)
            .map(|i| {
                (
                    format!("id-{}", i),
                    Some("a".repeat(1000)),
                    Some(format!("https://example.com/{}", i)),
                    "2024-03-25".to_string(),
                )
            })
            .collect();

        let (system, user) = build_prompt(&items);
        assert!(!system.is_empty());
        assert!(user.contains("[0]"));
        assert!(user.contains("[4]"));
        assert!(user.contains(&"a".repeat(500)));
        assert!(!user.contains(&"a".repeat(501)));
    }

    #[test]
    fn test_build_prompt_no_text() {
        let items = vec![("id-0".to_string(), None, None, "2024-03-25".to_string())];
        let (_system, user) = build_prompt(&items);
        assert!(user.contains("[无文本]"));
    }

    #[test]
    fn test_build_prompt_short_text_no_truncation() {
        let items = vec![(
            "id-0".to_string(),
            Some("短文本".to_string()),
            None,
            "2024-03-25".to_string(),
        )];
        let (_system, user) = build_prompt(&items);
        assert!(user.contains("短文本"));
        assert!(!user.contains("..."));
    }

    #[test]
    fn test_validate_radar_report() {
        let json = r#"{
          "meta": {
            "date_range": "2026-03-21 至 2026-04-05",
            "total_items": 65,
            "active_days": 12,
            "annotated_items": 34,
            "annotation_rate": "52%",
            "source_count": 7
          },
          "at_a_glance": [
            { "text": "洞察 1", "highlight": "重点" }
          ],
          "info_diet": {
            "sources": [
              { "name": "WeChat", "count": 24, "percent": 36.9, "color": "wechat" }
            ],
            "depth_ratio": { "deep": 53.8, "shallow": 46.2, "label": "深度长文 54% / 碎片 46%" },
            "dominant_topic": { "name": "AI工具链", "percent": 46.2, "label": "重度偏食" },
            "language_ratio": { "chinese": 76.9, "english": 23.1 },
            "alert": "警告"
          },
          "subconscious": [
            { "title": "收藏即掌握幻觉", "body": "说明", "evidence_count": 19 }
          ],
          "graveyard": {
            "forgotten_count": 31,
            "forgotten_percent": 47.7,
            "alert": "提醒",
            "top_picks": [
              { "rank": 1, "title": "值得重读", "reason": "为什么重读", "tags": ["AI"], "source": "WeChat", "date": "03-23" }
            ]
          },
          "blind_spots": [
            { "title": "商业模型盲区", "body": "说明" }
          ],
          "actions": [
            { "icon": "🔧", "title": "行动", "desc": "描述", "ref": "关联内容", "time": "90分钟" }
          ],
          "heatmap": [
            { "date": "03/21", "count": 4, "is_peak": false }
          ],
          "topic_cloud": [
            { "name": "AI工具链", "percent": 46.2 }
          ],
          "verdict": {
            "text": "总结",
            "highlights": ["重点"]
          },
          "footer": {
            "date_range": "03-21~04-05",
            "total": 65,
            "active_days": 12,
            "total_days": 16
          }
        }"#;
        let result = validate_radar_report(json);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_radar_report_requires_actions_ref() {
        let json = r#"{
          "meta": {
            "date_range": "2026-03-21 至 2026-04-05",
            "total_items": 1,
            "active_days": 1,
            "annotated_items": 1,
            "annotation_rate": "100%",
            "source_count": 1
          },
          "at_a_glance": [
            { "text": "洞察", "highlight": "重点" }
          ],
          "info_diet": {
            "sources": [
              { "name": "WeChat", "count": 1, "percent": 100.0, "color": "wechat" }
            ],
            "depth_ratio": { "deep": 100.0, "shallow": 0.0, "label": "深度长文 100% / 碎片 0%" },
            "dominant_topic": { "name": "AI工具链", "percent": 100.0, "label": "重度偏食" },
            "alert": "提醒"
          },
          "subconscious": [
            { "title": "标题", "body": "说明", "evidence_count": 1 }
          ],
          "graveyard": {
            "alert": "提醒",
            "top_picks": [
              { "rank": 1, "title": "重读", "reason": "原因", "tags": ["AI"] }
            ]
          },
          "blind_spots": [
            { "title": "盲区", "body": "说明" }
          ],
          "actions": [
            { "icon": "🔧", "title": "行动", "desc": "描述", "ref": "", "time": "15分钟" }
          ],
          "heatmap": [
            { "date": "03/21", "count": 1, "is_peak": true }
          ],
          "topic_cloud": [
            { "name": "AI工具链", "percent": 100.0 }
          ],
          "verdict": {
            "text": "总结",
            "highlights": ["重点"]
          },
          "footer": {
            "date_range": "03-21~03-21",
            "total": 1,
            "active_days": 1,
            "total_days": 1
          }
        }"#;

        let result = validate_radar_report(json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("actions.ref"));
    }

    #[test]
    fn test_process_dashscope_sse_event_rejects_invalid_json() {
        let mut pending_data_lines =
            vec![r#"{"choices":[{"delta":{"reasoning_content":"先思考"}}"#.to_string()];
        let mut content_acc = String::new();
        let mut reasoning_acc = String::new();

        let result = process_dashscope_sse_event(
            &mut pending_data_lines,
            &mut content_acc,
            &mut reasoning_acc,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_process_dashscope_sse_event_handles_done_and_json() {
        let mut pending_data_lines = vec![
            r#"{"choices":[{"delta":{"reasoning_content":"先思考","content":"最终答案"}}]}"#
                .to_string(),
        ];
        let mut content_acc = String::new();
        let mut reasoning_acc = String::new();

        let done = process_dashscope_sse_event(
            &mut pending_data_lines,
            &mut content_acc,
            &mut reasoning_acc,
        )
        .unwrap();

        assert!(!done);
        assert_eq!(reasoning_acc, "先思考");
        assert_eq!(content_acc, "最终答案");

        let mut done_lines = vec!["[DONE]".to_string()];
        let done =
            process_dashscope_sse_event(&mut done_lines, &mut content_acc, &mut reasoning_acc)
                .unwrap();
        assert!(done);
    }
}
