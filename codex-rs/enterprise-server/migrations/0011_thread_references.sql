CREATE TABLE IF NOT EXISTS enterprise_thread_references (
    reference_id UUID PRIMARY KEY,
    owner_user_id UUID NOT NULL REFERENCES enterprise_users(user_id) ON DELETE CASCADE,
    source_thread_id TEXT NOT NULL REFERENCES enterprise_sessions(session_id) ON DELETE CASCADE,
    target_thread_id TEXT NOT NULL REFERENCES enterprise_sessions(session_id) ON DELETE CASCADE,
    source_output_id UUID REFERENCES enterprise_outputs(output_id) ON DELETE SET NULL,
    output_id UUID REFERENCES enterprise_outputs(output_id) ON DELETE SET NULL,
    reference_type TEXT NOT NULL CHECK (reference_type IN ('transcript_export', 'ai_summary', 'handoff', 'artifact_import')),
    knowledge_origin TEXT NOT NULL CHECK (knowledge_origin IN ('user_generated', 'ai_generated')),
    status TEXT NOT NULL CHECK (status IN ('pending', 'completed', 'failed')),
    metadata_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS enterprise_thread_references_owner_target_idx
    ON enterprise_thread_references(owner_user_id, target_thread_id, created_at DESC);

CREATE INDEX IF NOT EXISTS enterprise_thread_references_owner_source_idx
    ON enterprise_thread_references(owner_user_id, source_thread_id, created_at DESC);
