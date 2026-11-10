-- おすそ分けで xAI（Grok）も使えるようにする（DESIGN.md 17.1）。おすそ分けする人が預けた xAI のキー。
-- base64(iv || AES-GCM の暗号文)。NULL は預けていない（そのコードでは xAI を中継しない）
ALTER TABLE issuers ADD COLUMN encrypted_xai_key TEXT;
