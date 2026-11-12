// API の Worker の入口（wrangler.jsonc の main）。外からの要求は index.ts の app が受ける。
// `Vault` は、管理画面の Worker（adminWorker.ts）だけが呼べる口（サービスバインディング。外からは呼べない）

import { WorkerEntrypoint } from "cloudflare:workers";
import app, { type Env } from "./index";
import { encrypt } from "./secrets";

/**
 * 開発者のキーを暗号化する（DESIGN.md 17.3）。暗号化の鍵（SHARE_KEY）は API の Worker だけが持ち、
 * 管理画面の Worker には渡さない。復号する口は作らない（管理画面はキーを読めない）
 */
export class Vault extends WorkerEntrypoint<Env> {
  seal(plain: string): Promise<string> {
    return encrypt(plain, this.env.SHARE_KEY);
  }

  /** 秘密の設定（wrangler）に入れたキーがあるか。管理画面のキーが断られたときの最後の頼み */
  fallbacks(): { qwen: boolean; openrouter: boolean } {
    return { qwen: !!this.env.BETA_QWEN_KEY, openrouter: !!this.env.BETA_OPENROUTER_KEY };
  }
}

export default app;
