# /// script
# requires-python = ">=3.12"
# dependencies = ["httpx"]
# ///
"""文字起こしの測定に使う音声を、ElevenLabs の音声生成で作る（DESIGN.md 3 章）。
台本（data/stt_script.json）の発話を、声ごとに 1 つずつ WAV（16kHz モノラル）にする。
合成音声ははっきり話すので、ふだんの配信の正確さではなく、用語を書けるかを比べるのに使う。
作ったあと、何本か聞いて、読み違いがないか確かめる。

usage: uv run tts_generate.py [--only N]
  ELEVENLABS_API_KEY を使う。出力は data/tts/声の名前/000.wav …（リポジトリには入れない）
"""

import json
import os
import sys
import wave
from pathlib import Path

import httpx

MODEL = "eleven_v4"
# 日本語の話者の声（アカウントの声の一覧にある名前の先頭）。くだけた声、落ち着いた低い声、高い声。
# ライブラリの声なので、API から使うには有料プランが要る
VOICES = {"aya": "Aya", "hijiri": "Hijiri", "kano": "Kano"}
RATE = 16000
HERE = Path(__file__).parent


def main() -> None:
    only = int(sys.argv[sys.argv.index("--only") + 1]) if "--only" in sys.argv else None
    lines = json.loads((HERE / "data/stt_script.json").read_text())[:only]
    client = httpx.Client(
        base_url="https://api.elevenlabs.io",
        headers={"xi-api-key": os.environ["ELEVENLABS_API_KEY"]},
        timeout=120,
    )
    listed = client.get("/v2/voices", params={"page_size": 100}).raise_for_status().json()["voices"]
    chars = 0
    for folder, name in VOICES.items():
        voice = next(v["voice_id"] for v in listed if v["name"].startswith(name))
        out = HERE / "data/tts" / folder
        out.mkdir(parents=True, exist_ok=True)
        for i, line in enumerate(lines):
            path = out / f"{i:03}.wav"
            if path.exists():
                continue
            res = client.post(
                f"/v1/text-to-speech/{voice}",
                params={"output_format": f"pcm_{RATE}"},
                json={"text": line["text"], "model_id": MODEL, "language_code": "ja"},
            )
            if res.status_code != 200:
                sys.exit(f"{folder} {i}: {res.status_code} {res.text[:300]}")
            with wave.open(str(path), "wb") as wav:
                wav.setnchannels(1)
                wav.setsampwidth(2)
                wav.setframerate(RATE)
                wav.writeframes(res.content)
            chars += len(line["text"])
        print(f"{folder}: {len(lines)} lines")
    print(f"generated {chars} characters")


if __name__ == "__main__":
    main()
