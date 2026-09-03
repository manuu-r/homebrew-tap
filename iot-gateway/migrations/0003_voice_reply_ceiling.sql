-- Raise the per-device voice reply ceiling.
--
-- devices.max_output_tokens defaults to 256, which clamps the effective voice
-- limit below the 512-token VOICE_MAX_OUTPUT_TOKENS. Lift existing rows so the
-- streamed, sentence-chunked replies can use the full budget. The column CHECK
-- already permits up to 4096.

UPDATE devices
   SET max_output_tokens = 512,
       updated_at = CURRENT_TIMESTAMP
 WHERE max_output_tokens < 512;
