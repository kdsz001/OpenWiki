-- Migration 019: Refresh exact model aliases without changing other providers.
UPDATE app_settings
SET value = 'qwen/qwen3.8-max-0902',
    updated_at = datetime('now')
WHERE value = 'qwen/qwen3.8-max'
  AND (
      key = 'ai_model_openrouter'
      OR (
          key = 'ai_model'
          AND EXISTS (
              SELECT 1 FROM app_settings
              WHERE key = 'ai_provider' AND value = 'openrouter'
          )
      )
  );

UPDATE app_settings
SET value = 'deepseek-flash',
    updated_at = datetime('now')
WHERE value IN ('deepseek-v4-flash', 'deepseek-v4-flash-vision-exp')
  AND (
      key = 'ai_model_deepseek'
      OR (
          key = 'ai_model'
          AND EXISTS (
              SELECT 1 FROM app_settings
              WHERE key = 'ai_provider' AND value = 'deepseek'
          )
      )
  );
