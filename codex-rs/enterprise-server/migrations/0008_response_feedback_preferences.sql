CREATE TABLE IF NOT EXISTS enterprise_response_feedback (
    feedback_id UUID PRIMARY KEY,
    owner_user_id UUID NOT NULL REFERENCES enterprise_users(user_id) ON DELETE CASCADE,
    session_id TEXT NOT NULL REFERENCES enterprise_sessions(session_id) ON DELETE CASCADE,
    message_id UUID NOT NULL REFERENCES enterprise_session_messages(message_id) ON DELETE CASCADE,
    rating TEXT NOT NULL CHECK (rating IN ('good', 'bad')),
    reason_tags JSONB NOT NULL DEFAULT '[]'::jsonb,
    comment TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (owner_user_id, message_id)
);

CREATE INDEX IF NOT EXISTS enterprise_response_feedback_owner_idx
    ON enterprise_response_feedback(owner_user_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS enterprise_user_response_preferences (
    owner_user_id UUID PRIMARY KEY REFERENCES enterprise_users(user_id) ON DELETE CASCADE,
    profile_summary TEXT NOT NULL DEFAULT '',
    positive_tags JSONB NOT NULL DEFAULT '[]'::jsonb,
    negative_tags JSONB NOT NULL DEFAULT '[]'::jsonb,
    sample_count BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
