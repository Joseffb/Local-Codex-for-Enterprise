-- Add user-visible chat transcript storage. This table is intentionally
-- separate from audit and receipt tables because prompts and model output must
-- not be copied into compliance evidence metadata.
CREATE TABLE IF NOT EXISTS enterprise_session_messages (
    message_id UUID PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES enterprise_sessions(session_id) ON DELETE CASCADE,
    owner_user_id UUID NOT NULL REFERENCES enterprise_users(user_id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('system', 'user', 'assistant')),
    label TEXT NOT NULL,
    text TEXT NOT NULL,
    sequence BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (session_id, sequence)
);

CREATE INDEX IF NOT EXISTS enterprise_session_messages_owner_idx
    ON enterprise_session_messages(owner_user_id, session_id, sequence);
