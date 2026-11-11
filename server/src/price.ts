// OpenAI と xAI の単価（2026-11）。アプリの crates/core/src/usage.rs と同じ表。
// おすそ分けでは、ここにないモデル（単価が分からない）は使わせない

/** キーを預けられる会社（おすそ分け） */
export type Provider = "openai" | "xai";

/** Qwen-Audio-3.1-ASR-Flash の単価（100 万トークンあたり。アプリの usage.rs と同じ） */
export const QWEN_AUDIO = { model: "qwen-audio-3.1-asr-flash", input: 0.15, output: 0.47 };
/** 途中の字幕（Qwen-Audio-3.1-ASR-Flash-Streaming）の単価（100 万トークンあたり。アプリの usage.rs と同じ） */
export const QWEN_LIVE = { model: "qwen-audio-3.1-asr-flash-streaming", input: 0.93, output: 0.7 };

/** OpenRouter が料金を答えに入れなかったときに数える単価（100 万トークンあたり。入れてくるのがふつう） */
export const OPENROUTER_FALLBACK = { input: 0.074, output: 0.295 };

type Price =
  | { provider: Provider; perMinute: number }
  | { input: number; cached: number; output: number };

const PRICES: Record<string, Price> = {
  "gpt-live-transcribe": { provider: "openai", perMinute: 0.017 },
  "gpt-transcribe": { provider: "openai", perMinute: 0.0045 },
  "grok-voice-transcribe-2.0": { provider: "xai", perMinute: 0.1 / 60 },
  // 100 万トークンあたり
  "gpt-6.1-terra": { input: 1.0, cached: 0.05, output: 5.0 },
  "gpt-6-luna": { input: 0.1, cached: 0.01, output: 0.5 },
};

/** その会社の音声のモデルの料金。そうでなければ undefined */
export function audioUsd(provider: Provider, model: string, seconds: number): number | undefined {
  const price = PRICES[model];
  return price && "perMinute" in price && price.provider === provider
    ? (seconds / 60) * price.perMinute
    : undefined;
}

export interface Tokens {
  /** キャッシュから読んだ分を含む */
  input: number;
  cached: number;
  output: number;
}

/** Responses API の料金。LLM のモデルでなければ undefined */
export function tokensUsd(model: string, t: Tokens): number | undefined {
  const price = PRICES[model];
  if (!price || "perMinute" in price) return undefined;
  const fresh = Math.max(t.input - t.cached, 0);
  return (fresh * price.input + t.cached * price.cached + t.output * price.output) / 1e6;
}

/** Responses API の応答の `usage` */
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

/**
 * WAV の長さ（秒）。アプリが送る形（PCM・16kHz・16bit・モノラル）だけを読み、それ以外は undefined。
 * 長さはヘッダーの値ではなく、実際に届いたバイト数から数える（ヘッダーを書き換えても安くならない）
 */
export function wavSeconds(bytes: ArrayBuffer): number | undefined {
  const view = new DataView(bytes);
  const tag = (at: number) =>
    at + 4 <= bytes.byteLength ? String.fromCharCode(...new Uint8Array(bytes, at, 4)) : "";
  if (tag(0) !== "RIFF" || tag(8) !== "WAVE") return undefined;
  let pcm16kMono = false;
  for (let at = 12; at + 8 <= bytes.byteLength; ) {
    const size = view.getUint32(at + 4, true);
    if (tag(at) === "fmt " && at + 24 <= bytes.byteLength) {
      pcm16kMono =
        view.getUint16(at + 8, true) === 1 &&
        view.getUint16(at + 10, true) === 1 &&
        view.getUint32(at + 12, true) === 16_000 &&
        view.getUint16(at + 22, true) === 16;
    }
    if (tag(at) === "data") return pcm16kMono ? (bytes.byteLength - at - 8) / 32_000 : undefined;
    at += 8 + size + (size % 2);
  }
  return undefined;
}

/** Realtime で受ける音声のレート（`appendSeconds` が数えられるのはこの PCM だけ） */
export const REALTIME_RATE = 24_000;

/** Realtime の `input_audio_buffer.append`（24kHz・16bit の PCM を base64）の長さ（秒） */
export function appendSeconds(base64: string): number {
  const padding = base64.endsWith("==") ? 2 : base64.endsWith("=") ? 1 : 0;
  const bytes = (base64.length * 3) / 4 - padding;
  return bytes / 2 / REALTIME_RATE;
}
