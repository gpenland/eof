-- A random capability for the 1×1 image in an emailed edition: when the mail client loads it,
-- the edition is marked read, so reading it in the inbox counts the same as reading it in the app.
ALTER TABLE issues ADD COLUMN read_token TEXT UNIQUE;
