/// System prompt for the weekly report generation AI call.
pub fn weekly_report_system_prompt(locale: &str) -> String {
    if crate::locale::is_english(locale) {
        r#"You are "OpenWiki" — a professional personal information analyst. Your job is to analyze the user's saved content from this week (text, web pages, image descriptions, etc.) and produce a high-value, prioritized weekly report.

## Core principles:
- Highlight change, not inventory: Tell the user what's new this week, not just what exists
- Suppress the routine, amplify the unexpected: Don't emphasize what the user already knows — surface surprises
- Every insight should be actionable: Pair information with a suggestion or connection
- Brevity first: The user should scan the summary in 10 seconds and read the full report in 1-2 minutes
- Filter noise aggressively: Ignore meaningless fragments (bare numbers, garbled text, context-free phrases) — do not include them in any section

## Your analytical capabilities:
1. Topic clustering: Group related content under common themes
2. Importance assessment: Judge significance by depth, timeliness, and user interest
3. Trend detection: Identify cross-content patterns and trends
4. Personalized recommendation: Emphasize content aligned with user preferences
5. Web content understanding: When the user saves a link, the full page text has already been fetched — analyze the actual content

## Section type rules (section_type):
Each section must be one of the following types:
- key_insight: Important findings the user might miss. relevance_score should be 0.8-1.0
- highlight: Notable achievements or discoveries this week. relevance_score should be 0.6-0.8
- trend: Patterns or trends spanning multiple items. relevance_score should be 0.5-0.7
- routine: Everyday, unremarkable content. relevance_score should be 0.1-0.4
- recommendation: Action items based on this week's content. relevance_score should be 0.7-0.9

## Quantity requirements:
- key_insight: At most 1-2 (only for truly important findings)
- highlight: 1-2
- trend: 0-2
- routine: 0 (do not generate routine sections — just ignore irrelevant content)
- recommendation: Exactly 1 (always provide an action item)
- Total: 3-6 sections

## Output requirements:
- Return pure JSON — no markdown code block markers
- Write all text in English
- summary: A concise overview in 2-3 sentences (30-60 words), stating the most important thing this week
- sections sorted by relevance_score descending
- Each section must include specific content references (content_ids)
- relevance_score must be between 0.0-1.0 — use the full range"#.to_string()
    } else {
        r#"你是「OpenWiki」——一个专业的个人信息分析助手。你的职责是分析用户本周保存的各类内容（文本、网页、图片描述等），生成一份高价值、有优先级的中文周报。

## 核心原则：
- 突出变化，而非罗列事实：告诉用户本周"发生了什么新变化"，而非"有哪些内容"
- 压制常规，放大异常：用户已知的常规信息不需要强调，意外的发现才有价值
- 每个洞察都应可操作：告知用户信息的同时，给出建议或关联
- 简洁至上：用户应在10秒内扫完摘要，1-2分钟读完全文
- 严格过滤噪音：忽略明显无意义的碎片内容（如纯数字、乱码、无上下文的短语），不要把它们写进任何 section

## 你的分析能力：
1. 主题聚类：将相关内容归纳到同一主题下
2. 重要性评估：根据内容深度、时效性和用户兴趣判断重要程度
3. 趋势发现：识别跨内容的关联模式和趋势
4. 个性化推荐：结合用户偏好突出重点内容
5. 网页内容理解：当用户保存链接时，已自动获取网页正文，请基于实际内容进行分析

## 板块分类规则（section_type）：
每个 section 必须指定以下类型之一：
- key_insight：需要关注的重要发现，用户可能会错过的关键信息。relevance_score 应在 0.8-1.0
- highlight：本周亮点，值得注意的成果或发现。relevance_score 应在 0.6-0.8
- trend：跨多条内容的趋势或模式。relevance_score 应在 0.5-0.7
- routine：常规、日常性质的内容，不需要特别关注。relevance_score 应在 0.1-0.4
- recommendation：基于本周内容的行动建议，告诉用户下一步该做什么。relevance_score 应在 0.7-0.9

## 数量要求：
- key_insight：最多 1-2 个（只有真正重要的才标记）
- highlight：1-2 个
- trend：0-2 个
- routine：0 个（不要生成 routine 类型的 section，无关内容直接忽略即可）
- recommendation：恰好 1 个（始终提供行动建议）
- 总计 3-6 个 sections

## 输出要求：
- 必须以纯JSON格式返回，不要包含markdown代码块标记
- 所有文字使用中文
- summary：2-3句话的精炼概述（50-80字），直接点明本周最重要的事
- sections 按 relevance_score 从高到低排列
- 每个 section 包含具体的内容引用（content_ids）
- relevance_score 必须在 0.0-1.0 之间，请充分利用整个范围"#.to_string()
    }
}

/// Build the user message for weekly report generation.
/// `content_summaries` - formatted list of this week's saved content
/// `user_interests` - text summary of user preference topics
pub fn weekly_report_user_message(
    content_summaries: &str,
    user_interests: &str,
    locale: &str,
) -> String {
    if crate::locale::is_english(locale) {
        let interests_section = if user_interests.is_empty() {
            "No preference data available — analyze all content equally.".to_string()
        } else {
            user_interests.to_string()
        };

        format!(
            r#"Analyze the following content saved by the user this week and generate a structured weekly report.

## User's saved content this week:
{content_summaries}

## User interest preferences:
{interests_section}

## Return strictly in the following JSON format (no markdown markers):
{{
  "summary": "Core overview of the week (2-3 sentences, 30-60 words, state the most important thing)",
  "sections": [
    {{
      "title": "Topic title",
      "body": "Detailed analysis of this topic (50-150 words), highlighting changes and key takeaways",
      "section_type": "key_insight",
      "relevance_score": 0.85,
      "content_ids": ["list of related content IDs"]
    }}
  ]
}}

Notes:
- section_type must be one of: key_insight, highlight, trend, recommendation (do not use routine)
- Ignore all fragmented, context-free, or unclassifiable content — do not mention them
- Sort sections by relevance_score descending
- The recommendation section should contain specific, actionable suggestions"#
        )
    } else {
        let interests_section = if user_interests.is_empty() {
            "暂无偏好数据，请均衡分析所有内容。".to_string()
        } else {
            user_interests.to_string()
        };

        format!(
            r#"请分析以下用户本周保存的内容，生成结构化周报。

## 用户本周保存的内容：
{content_summaries}

## 用户兴趣偏好：
{interests_section}

## 请严格按照以下JSON格式返回（不要添加任何markdown标记）：
{{
  "summary": "本周核心概述（2-3句话，50-80字，直接点明最重要的事）",
  "sections": [
    {{
      "title": "主题标题",
      "body": "该主题的详细分析（100-300字），突出变化和关键要点",
      "section_type": "key_insight",
      "relevance_score": 0.85,
      "content_ids": ["相关内容的id列表"]
    }}
  ]
}}

注意：
- section_type 必须是 key_insight、highlight、trend、recommendation 之一（不要使用 routine 类型）
- 忽略所有零碎、无上下文、无法归类的内容片段，不要提及它们
- sections 按 relevance_score 从高到低排列
- recommendation 类型的 section 应包含具体可操作的建议"#
        )
    }
}

/// Prompt for summarizing a single content item before feeding into the weekly report.
/// Used to truncate and summarize long content items.
pub fn content_summarize_prompt(
    raw_text: &str,
    content_type: &str,
    source_app: &str,
    locale: &str,
) -> String {
    if crate::locale::is_english(locale) {
        format!(
            r#"Summarize the core information of the following content in under 30 words.

Source app: {source_app}
Content type: {content_type}
Original text:
{raw_text}

Return only the summary text — no prefixes or formatting."#
        )
    } else {
        format!(
            r#"请用50字以内简要概括以下内容的核心信息。

来源应用：{source_app}
内容类型：{content_type}
原文：
{raw_text}

请直接返回概括文字，不要添加任何前缀或格式标记。"#
        )
    }
}

/// Prompt for extracting topic keywords from a piece of content.
/// Used by the preference engine when user marks content as "interested".
pub fn topic_extraction_prompt(text: &str, locale: &str) -> String {
    if crate::locale::is_english(locale) {
        format!(
            r#"Extract 3-5 core topic keywords from the following text that describe its main subject areas.

Text:
{text}

Requirements:
- Each keyword should be 1-4 words
- Separate with commas
- Return only the keyword list — no explanations

Example output: artificial intelligence,deep learning,natural language processing"#
        )
    } else {
        format!(
            r#"请从以下文本中提取3-5个核心主题关键词，用于描述该内容的主要话题领域。

文本：
{text}

要求：
- 每个关键词2-6个字
- 用逗号分隔
- 只返回关键词列表，不要添加任何解释

示例输出：人工智能,深度学习,自然语言处理"#
        )
    }
}

/// Prompt for clustering content items into topic groups.
pub fn topic_clustering_prompt(content_list: &str, locale: &str) -> String {
    if crate::locale::is_english(locale) {
        format!(
            r#"Cluster the following content items by topic.

## Content list:
{content_list}

## Requirements:
Return the clustering result in JSON format — no markdown markers:
{{
  "clusters": [
    {{
      "topic": "Topic name",
      "content_ids": ["id1", "id2"],
      "keywords": ["keyword1", "keyword2"]
    }}
  ]
}}"#
        )
    } else {
        format!(
            r#"请将以下内容按主题进行聚类分组。

## 内容列表：
{content_list}

## 要求：
请以JSON格式返回聚类结果，不要添加markdown标记：
{{
  "clusters": [
    {{
      "topic": "主题名称",
      "content_ids": ["id1", "id2"],
      "keywords": ["关键词1", "关键词2"]
    }}
  ]
}}"#
        )
    }
}

/// Format a single content item for inclusion in the weekly report prompt.
pub fn format_content_item(
    id: &str,
    content_type: &str,
    source_app: &str,
    captured_at: &str,
    text_preview: &str,
    locale: &str,
) -> String {
    if crate::locale::is_english(locale) {
        format!(
            "- [ID: {id}] [{content_type}] from \"{source_app}\" ({captured_at}): {text_preview}"
        )
    } else {
        format!("- [ID: {id}] [{content_type}] 来自「{source_app}」({captured_at}): {text_preview}")
    }
}
