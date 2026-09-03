-- Video prompts are user content and are not required for polling or billing.
-- Remove historical copies now that new writes discard them before persistence.
UPDATE video_tasks
SET prompt = NULL
WHERE prompt IS NOT NULL;
