// OpenAI の単価（2026-11）。アプリの crates/core/src/usage.rs と同じ表。
// おすそ分けでは、ここにないモデル（単価が分からない）は使わせない

type Price = { perMinute: number } | { input: number; cached: number; output: number };

const PRICES: Record<string, Price> = {
  "gpt-live-transcribe": { perMinute: 0.017 },
  "gpt-transcribe": { perMinute: 0.0045 },
  // 100 万トークンあたり
  "gpt-6.1-terra": { input: 1.0, cached: 0.05, output: 5.0 },
};

/** 音声の料金。音声のモデルでなければ undefined */
export function audioUsd(model: string, seconds: number): number | undefined {
  const price = PRICES[model];
  return price && "perMinute" in price ? (seconds / 60) * price.perMinute : undefined;
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

/** WAV の長さ（秒）。読めなければ undefined */
export function wavSeconds(bytes: ArrayBuffer): number | undefined {
  const view = new DataView(bytes);
  const tag = (at: number) =>
    String.fromCharCode(...new Uint8Array(bytes, at, Math.min(4, bytes.byteLength - at)));
  if (bytes.byteLength < 12 || tag(0) !== "RIFF" || tag(8) !== "WAVE") return undefined;
  let byteRate: number | undefined;
  for (let at = 12; at + 8 <= bytes.byteLength; ) {
    const size = view.getUint32(at + 4, true);
    if (tag(at) === "fmt " && at + 16 <= bytes.byteLength) byteRate = view.getUint32(at + 16, true);
    if (tag(at) === "data") return byteRate ? size / byteRate : undefined;
    at += 8 + size + (size % 2);
  }
  return undefined;
}

/** Realtime の `input_audio_buffer.append`（24kHz・16bit の PCM を base64）の長さ（秒） */
export function appendSeconds(base64: string): number {
  const padding = base64.endsWith("==") ? 2 : base64.endsWith("=") ? 1 : 0;
  const bytes = (base64.length * 3) / 4 - padding;
  return bytes / 2 / 24_000;
}
