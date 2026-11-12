# /// script
# requires-python = ">=3.12"
# dependencies = ["httpx"]
# ///
"""OpenRouter の文字起こしのモデル（MAI-Transcribe 2 など）の確定の文字起こしを、
stt_eval_openai.py と同じ指標（文字誤り率）で測る。用語は渡せない（渡す欄がない）。

usage: uv run stt_eval_openrouter.py data/sample01.m4a data/sample01.txt MODEL... [--repeat N]
  OPENROUTER_API_KEY を使う。料金は答えの `usage.cost`
"""

import json
import os
import sys
import time
from pathlib import Path

import httpx

from stt_eval_xai_qwen import cer, load_mp3


def transcribe(model: str, audio: bytes) -> dict:
    t0 = time.monotonic()
    r = httpx.post(
        "https://openrouter.ai/api/v1/audio/transcriptions",
        headers={"Authorization": f"Bearer {os.environ['OPENROUTER_API_KEY']}"},
        data={"model": model, "language": "ja"},
        files={"file": ("audio.mp3", audio, "audio/mpeg")},
        timeout=120,
    )
    if r.is_error:
        raise RuntimeError(f"{r.status_code} {r.text[:300]}")
    body = r.json()
    return {
        "text": body["text"],
        "latency_s": round(time.monotonic() - t0, 2),
        "usd": body.get("usage", {}).get("cost", 0),
    }


def main() -> None:
    args = sys.argv[1:]
    repeat = 3
    if "--repeat" in args:
        i = args.index("--repeat")
        repeat = int(args[i + 1])
        del args[i : i + 2]
    path, ref = Path(args[0]), Path(args[1]).read_text().strip()
    audio = load_mp3(path)
    results: dict[str, list] = {}
    for model in args[2:]:
        runs = results.setdefault(model, [])
        for _ in range(repeat):
            try:
                r = transcribe(model, audio)
                r["cer"] = round(cer(ref, r["text"]), 4)
            except Exception as e:
                r = {"text": "", "error": str(e)[:300]}
            runs.append(r)
    out = Path(__file__).parent / "results" / f"{path.stem}.openrouter.json"
    out.write_text(json.dumps(results, ensure_ascii=False, indent=2))
    for name, runs in results.items():
        print(f"## {name}")
        for r in runs:
            extra = r.get("error") or f"CER {r['cer']:.1%}  {r['latency_s']}s  ${r['usd']:.5f}"
            print(f"  {extra}\n  {r['text']}")
        print()


if __name__ == "__main__":
    main()
