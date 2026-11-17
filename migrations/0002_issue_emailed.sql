-- When the edition was emailed to the owner. Set once when today's edition is first published
-- (republishing the same day keeps it, so corrections don't re-send); a manual send updates it.
ALTER TABLE issues ADD COLUMN emailed_at TIMESTAMPTZ;
