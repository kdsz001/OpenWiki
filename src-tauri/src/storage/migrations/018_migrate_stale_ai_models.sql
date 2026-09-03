-- Migration 018: Replace retired preset model IDs with supported equivalents.
UPDATE app_settings
SET value = CASE value
        WHEN 'claude-sonnet-4-20250514' THEN 'claude-sonnet-4-6'
        WHEN 'claude-opus-4-20250514' THEN 'claude-opus-4-8'
        WHEN 'claude-3-5-haiku-20241022' THEN 'claude-haiku-4-5-20251001'
    END,
    updated_at = datetime('now')
WHERE key IN ('ai_model', 'ai_model_anthropic')
  AND value IN (
      'claude-sonnet-4-20250514',
      'claude-opus-4-20250514',
      'claude-3-5-haiku-20241022'
  );

UPDATE app_settings
SET value = CASE value
        WHEN 'gpt-5.1-codex-mini' THEN 'gpt-5.6-terra'
        ELSE 'gpt-5.6'
    END,
    updated_at = datetime('now')
WHERE key IN ('ai_model', 'ai_model_openai')
  AND value IN (
      'gpt-5.2-codex',
      'gpt-5.1-codex-max',
      'gpt-5.1-codex',
      'gpt-5.1-codex-mini'
  );

UPDATE app_settings
SET value = CASE value
        WHEN 'google/gemini-3-pro-preview' THEN 'google/gemini-3.1-pro-preview'
        WHEN 'deepseek/deepseek-v3.2-speciale' THEN 'deepseek/deepseek-v4-flash'
        WHEN 'x-ai/grok-4.1-fast' THEN 'x-ai/grok-4.20'
        ELSE 'openrouter/free'
    END,
    updated_at = datetime('now')
WHERE key IN ('ai_model', 'ai_model_openrouter')
  AND value IN (
      'nousresearch/hermes-3-llama-3.1-405b:free',
      'qwen/qwen3-coder:free',
      'openai/gpt-oss-120b:free',
      'qwen/qwen3-next-80b-a3b-instruct:free',
      'meta-llama/llama-3.3-70b-instruct:free',
      'minimax/minimax-m2.5:free',
      'z-ai/glm-4.5-air:free',
      'google/gemma-3-27b-it:free',
      'nvidia/nemotron-3-nano-30b-a3b:free',
      'openai/gpt-oss-20b:free',
      'arcee-ai/trinity-large-preview:free',
      'google/gemini-3-pro-preview',
      'deepseek/deepseek-v3.2-speciale',
      'x-ai/grok-4.1-fast'
  );
