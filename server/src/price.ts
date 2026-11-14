// ベータで中継するモデルの単価（2026-11）。アプリの crates/core/src/usage.rs と同じ値

/** Qwen-Audio-3.1-ASR-Flash の単価（100 万トークンあたり。アプリの usage.rs と同じ） */
export const QWEN_AUDIO = { model: "qwen-audio-3.1-asr-flash", input: 0.15, output: 0.47 };
/** 途中の字幕（Qwen-Audio-3.1-ASR-Flash-Streaming）の単価（100 万トークンあたり。アプリの usage.rs と同じ） */
export const QWEN_LIVE = { model: "qwen-audio-3.1-asr-flash-streaming", input: 0.93, output: 0.7 };

/** OpenRouter が料金を答えに入れなかったときに数える単価（100 万トークンあたり。入れてくるのがふつう） */
export const OPENROUTER_FALLBACK = { input: 0.074, output: 0.295 };

export interface Tokens {
  /** キャッシュから読んだ分を含む */
  input: number;
  cached: number;
  output: number;
}

/** 応答の `usage`（トークンの数） */
export function tokensOf(usage: unknown): Tokens {
  const u = (usage ?? {}) as {
    input_tokens?: number;
    output_tokens?: number;
    input_tokens_details?: { cached_tokens?: number };
  };
  return {
    input: u.input_tokens ?? 0,
    cached: u.input_tokens_details?.cached_tokens ?? 0,
    output: u.output_tokens ?? 0,
  };
}
