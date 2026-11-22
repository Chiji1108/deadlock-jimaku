// crates/core/src/settings.rs の `catalog` が作る（`cargo test`）。手で書き換えない

/** 会社。鍵が要る順 */
export const PROVIDERS = [
  {
    "id": "openai",
    "name": "OpenAI"
  },
  {
    "id": "xai",
    "name": "xAI"
  },
  {
    "id": "qwen",
    "name": "Qwen"
  },
  {
    "id": "openrouter",
    "name": "OpenRouter"
  },
  {
    "id": "typesafe",
    "name": "TypeSafe AI"
  }
] as const;

/** 工程の選択肢ごとの、使う会社（無料なら null） */
export const ENGINE_PROVIDERS = {
  "live": {
    "chrome": null,
    "server": null,
    "gptLiveTranscribe": "openai",
    "qwenAudioStreaming": "qwen",
    "haasJa": null,
    "whisper": null,
    "off": null
  },
  "transcription": {
    "chrome": null,
    "server": null,
    "gptTranscribe": "openai",
    "grokVoiceTranscribe": "xai",
    "qwenAudio": "qwen",
    "maiTranscribe2": "openrouter",
    "haasJa": null,
    "reazonSpeech": null,
    "whisperTurbo": null,
    "whisperLarge": null
  },
  "terms": {
    "dictionary": null,
    "server": null,
    "geminiNano": null,
    "gemma": null,
    "decisions": "openai",
    "jev": "typesafe",
    "drexMax": "openrouter",
    "decision1": "openrouter",
    "pplxDecider": "openrouter"
  },
  "translation": {
    "chromeTranslator": null,
    "gemma": null,
    "server": null,
    "qwenMtFlash": "qwen",
    "hyMt2": "openrouter",
    "qwen38Flash": "qwen",
    "gpt6LunaMt": "openai",
    "gpt61Terra": "openai"
  },
  "furigana": {
    "dictionary": null,
    "server": null,
    "decider": "openrouter",
    "luna": "openai",
    "llm": "openai"
  }
} as const;
