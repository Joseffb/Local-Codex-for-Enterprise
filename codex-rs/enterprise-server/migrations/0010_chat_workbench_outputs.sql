ALTER TABLE enterprise_sessions
    ADD COLUMN IF NOT EXISTS last_opened_at TIMESTAMPTZ;

UPDATE enterprise_sessions
SET last_opened_at = COALESCE(last_opened_at, updated_at, created_at)
WHERE last_opened_at IS NULL;

CREATE INDEX IF NOT EXISTS enterprise_sessions_active_owner_last_opened_idx
    ON enterprise_sessions(owner_user_id, last_opened_at DESC)
    WHERE deleted_at IS NULL;

ALTER TABLE enterprise_session_messages
    ADD COLUMN IF NOT EXISTS retry_of_message_id UUID REFERENCES enterprise_session_messages(message_id) ON DELETE SET NULL;

ALTER TABLE enterprise_session_messages
    ADD COLUMN IF NOT EXISTS supersedes_message_id UUID REFERENCES enterprise_session_messages(message_id) ON DELETE SET NULL;

ALTER TABLE enterprise_session_messages
    ADD COLUMN IF NOT EXISTS context_cutoff_message_id UUID REFERENCES enterprise_session_messages(message_id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS enterprise_session_messages_retry_idx
    ON enterprise_session_messages(retry_of_message_id)
    WHERE retry_of_message_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS enterprise_session_messages_supersedes_idx
    ON enterprise_session_messages(supersedes_message_id)
    WHERE supersedes_message_id IS NOT NULL;
