-- NIP-BR kinds 30623 / 30624 / 40030 carry the owner's include-set, reporter
-- watermarks, and the daily digest body (private channels and DMs). Exclude
-- them from full-text search without changing the search policy of existing
-- installations. Fresh installs are already safe via the positive allowlist
-- from migration 0008; this closes the brownfield gap where a populated
-- database still runs the legacy negative skip-set and would tokenize the
-- digest into search_tsv.
--
-- Same shape as 0014 / 0036: PostgreSQL cannot alter a generated expression
-- in place, so capture the current expression, drop the column, and re-add
-- it wrapped with the new exclusion. Every other kind keeps whatever policy
-- the database had before.
--
-- Operational cost: DROP COLUMN + ADD ... GENERATED ... STORED rewrites the
-- events heap and rebuilds the GIN index under ACCESS EXCLUSIVE. Schedule a
-- window on large brownfield databases. This is the relay-leg migration for
-- the Summary rollout.
DO $$
DECLARE
    existing_expression TEXT;
BEGIN
    SELECT pg_get_expr(d.adbin, d.adrelid)
      INTO existing_expression
      FROM pg_attrdef d
      JOIN pg_attribute a
        ON a.attrelid = d.adrelid
       AND a.attnum = d.adnum
     WHERE d.adrelid = 'events'::regclass
       AND a.attname = 'search_tsv';

    IF existing_expression IS NULL THEN
        RAISE EXCEPTION 'events.search_tsv generated expression not found';
    END IF;

    ALTER TABLE events DROP COLUMN search_tsv;
    EXECUTE format(
        'ALTER TABLE events ADD COLUMN search_tsv TSVECTOR GENERATED ALWAYS AS (CASE WHEN kind IN (30623, 30624, 40030) THEN NULL::tsvector ELSE (%s) END) STORED',
        existing_expression
    );
    CREATE INDEX idx_events_search_tsv ON events USING GIN (search_tsv);
END $$;
