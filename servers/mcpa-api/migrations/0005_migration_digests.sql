-- SPDX-License-Identifier: FSL-1.1-ALv2
--
-- Record the exact embedded bytes used for every applied migration. The
-- runner backfills earlier versions and makes this column required before
-- committing the complete pending set.

ALTER TABLE mcpa_schema_migrations
    ADD COLUMN digest text CHECK (digest ~ '^[0-9a-f]{64}$');
