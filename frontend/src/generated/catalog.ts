// crates/core/src/settings.rs の `catalog` が作る（`cargo test`）。手で書き換えない

/** 会社。鍵が要る順 */
export const PROVIDERS = [
  {
    "id": "openai",
    "name": "OpenAI",
    "relay": "gift"
  },
  {
    "id": "xai",
    "name": "xAI",
    "relay": "gift"
  },
  {
    "id": "qwen",
    "name": "Qwen",
    "relay": "beta"
  },
  {
    "id": "openrouter",
    "name": "OpenRouter",
    "relay": "beta"
  },
  {
    "id": "typesafe",
    "name": "TypeSafe AI",
    "relay": null
  }
] as const;

/** 工程の選択肢ごとの、使う会社（無料なら null） */
export const ENGINE_PROVIDERS = {
  "live": {
    "chrome": null,
    "gptLiveTranscribe": "openai",
    "qwenAudioStreaming": "qwen",
    "haasJa": null,
    "off": null
  },
  "transcription": {
    "chrome": null,
    "gptTranscribe": "openai",
    "grokVoiceTranscribe": "xai",
    "qwenAudio": "qwen",
    "maiTranscribe2": "openrouter",
    "haasJa": null
  },
  "terms": {
    "dictionary": null,
    "decisions": "openai",
    "jev": "typesafe",
    "drexMax": "openrouter",
    "decision1": "openrouter",
    "pplxDecider": "openrouter"
  },
  "translation": {
    "chromeTranslator": null,
    "qwenMtFlash": "qwen",
    "hyMt2": "openrouter",
    "gpt61Terra": "openai"
  },
  "furigana": {
    "lindera": null,
    "llm": "openai"
  }
} as const;
