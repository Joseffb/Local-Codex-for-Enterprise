ALTER TABLE enterprise_context_documents
    ADD COLUMN IF NOT EXISTS relative_path TEXT,
    ADD COLUMN IF NOT EXISTS content_bytes BYTEA,
    ADD COLUMN IF NOT EXISTS content_type TEXT NOT NULL DEFAULT 'text/markdown',
    ADD COLUMN IF NOT EXISTS file_size_bytes BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS file_kind TEXT NOT NULL DEFAULT 'document',
    ADD COLUMN IF NOT EXISTS loadable BOOLEAN NOT NULL DEFAULT true,
    ADD COLUMN IF NOT EXISTS is_system_file BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN IF NOT EXISTS source_type TEXT NOT NULL DEFAULT 'manual',
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;

UPDATE enterprise_context_documents
SET relative_path = filename
WHERE relative_path IS NULL;

UPDATE enterprise_context_documents
SET content_bytes = convert_to('', 'UTF8')
WHERE content_bytes IS NULL;

UPDATE enterprise_context_documents
SET file_size_bytes = length(content_bytes)
WHERE file_size_bytes = 0;

ALTER TABLE enterprise_context_documents
    ALTER COLUMN relative_path SET NOT NULL,
    ALTER COLUMN content_bytes SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'enterprise_context_documents_file_kind_check'
    ) THEN
        ALTER TABLE enterprise_context_documents
            ADD CONSTRAINT enterprise_context_documents_file_kind_check
            CHECK (file_kind IN ('document', 'bundle', 'asset'));
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'enterprise_context_documents_source_type_check'
    ) THEN
        ALTER TABLE enterprise_context_documents
            ADD CONSTRAINT enterprise_context_documents_source_type_check
            CHECK (source_type IN ('manual', 'upload', 'import'));
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'enterprise_context_documents_file_size_check'
    ) THEN
        ALTER TABLE enterprise_context_documents
            ADD CONSTRAINT enterprise_context_documents_file_size_check
            CHECK (file_size_bytes >= 0 AND file_size_bytes <= 10485760);
    END IF;
END $$;

CREATE UNIQUE INDEX IF NOT EXISTS enterprise_context_documents_active_path_idx
    ON enterprise_context_documents(pack_id, relative_path)
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS enterprise_context_documents_active_load_idx
    ON enterprise_context_documents(pack_id, load_order)
    WHERE deleted_at IS NULL AND loadable = true;
