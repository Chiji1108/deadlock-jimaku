// crates/core/src/settings.rs の `catalog` が作る（`cargo test`）。手で書き換えない

/** 会社。鍵が要る順 */
export const PROVIDERS = [
  {
    "id": "openai",
    "name": "OpenAI",
    "giftable": true
  },
  {
    "id": "xai",
    "name": "xAI",
    "giftable": true
  },
  {
    "id": "qwen",
    "name": "Qwen",
    "giftable": false
  },
  {
    "id": "openrouter",
    "name": "OpenRouter",
    "giftable": false
  },
  {
    "id": "typesafe",
    "name": "TypeSafe AI",
    "giftable": false
  }
] as const;

/** 工程の選択肢ごとの、使う会社（無料なら null） */
export const ENGINE_PROVIDERS = {
  "live": {
    "chrome": null,
    "gptLiveTranscribe": "openai",
    "qwenAudioStreaming": "qwen",
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
    "gpt61Terra": "openai",
    "gpt6Luna": "openai"
  },
  "furigana": {
    "lindera": null,
    "llm": "openai"
  }
} as const;
