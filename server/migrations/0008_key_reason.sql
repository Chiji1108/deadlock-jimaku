-- キーが断られた理由（外が返した本文の先頭）を覚える。管理画面に出す
ALTER TABLE `beta_keys` ADD `failed_reason` text;
