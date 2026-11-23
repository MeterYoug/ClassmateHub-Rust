ALTER TABLE seat_plan ADD COLUMN source TEXT NOT NULL DEFAULT 'offline' CHECK(source IN ('offline', 'ai'));
