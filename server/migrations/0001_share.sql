-- おすそ分け（DESIGN.md 17.1）。キーとコードと合言葉は、そのものを保存しない（キーは暗号化、ほかはハッシュ）

-- おすそ分けを有効にした人。同じキーで有効にし直したら同じ行（合言葉だけ作り直す）
CREATE TABLE issuers (
  id TEXT PRIMARY KEY,
  admin_token_hash TEXT NOT NULL UNIQUE,
  key_hash TEXT NOT NULL UNIQUE,
  -- base64(iv || AES-GCM の暗号文)
  encrypted_key TEXT NOT NULL,
  created_at INTEGER NOT NULL
);

CREATE TABLE codes (
  id TEXT PRIMARY KEY,
  code_hash TEXT NOT NULL UNIQUE,
  issuer_id TEXT NOT NULL REFERENCES issuers (id) ON DELETE CASCADE,
  memo TEXT NOT NULL DEFAULT '',
  -- ミリ秒。NULL は期限なし
  expires_at INTEGER,
  revoked INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);

CREATE INDEX codes_issuer ON codes (issuer_id);

-- コードで使われた額（月ごと。月は UTC の "2026-11"）
CREATE TABLE code_usage (
  code_id TEXT NOT NULL REFERENCES codes (id) ON DELETE CASCADE,
  month TEXT NOT NULL,
  usd REAL NOT NULL DEFAULT 0,
  PRIMARY KEY (code_id, month)
);
