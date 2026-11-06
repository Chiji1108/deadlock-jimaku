-- コードごとの 1 日の上限（DESIGN.md 17.1）。日は UTC。今の日の分だけを持ち、日が変わったら上書きする

-- ドル。NULL は既定の額（db.ts の DEFAULT_DAILY_LIMIT_USD）で、今あるコードもこれになる
ALTER TABLE codes ADD COLUMN daily_limit_usd REAL;
-- day_usd を数えている日（"2026-11-06"）
ALTER TABLE codes ADD COLUMN day TEXT;
ALTER TABLE codes ADD COLUMN day_usd REAL NOT NULL DEFAULT 0;
