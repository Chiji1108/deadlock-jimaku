# /// script
# requires-python = ">=3.12"
# dependencies = ["httpx", "websockets"]
# ///
"""ElevenLabs の音声認識（Scribe v2 Realtime / Scribe v2）の日本語精度を、stt_eval_openai.py と同じ指標で測る。
用語は keyterms で渡す（バッチは 1000 件・50 文字まで、Realtime は 50 件・20 文字までなのでヒーロー名だけ）。
Realtime は発話の区切りをサーバーの VAD に任せ、確定した文（committed_transcript）をつなげる。
`no_verbatim` は言いよどみ（「い--」「うん」）を除く設定。

usage: uv run stt_eval_elevenlabs.py data/sample01.m4a data/sample01.txt
"""

import asyncio
import base64
import json
import os
import sys
import time
from pathlib import Path
from urllib.parse import urlencode

import httpx
import websockets

from stt_eval_openai import CHUNK_MS, SAMPLE_RATE, cer, load_glossary, load_keywords, load_pcm

REALTIME_MODEL = "scribe_v2_realtime"
BATCH_MODEL = "scribe_v2"


def keyterms(words: list[str], limit: int = 1000, max_len: int = 50) -> list[str]:
    """keyterms の制限: 文字数、5 語まで、<>{}[]\\ を含まない、件数"""
    ok = [w for w in words if len(w) <= max_len and len(w.split()) <= 5 and not set(w) & set("<>{}[]\\")]
    return ok[:limit]


def hero_names() -> list[str]:
    entities = load_glossary()
    return [e["names"]["ja"] for e in entities.values() if e["kind"] == "hero" and e["names"].get("ja")]


async def realtime(pcm: bytes, key: str, terms: list[str], no_verbatim: bool = False) -> dict:
    query = [("model_id", REALTIME_MODEL), ("audio_format", f"pcm_{SAMPLE_RATE}"), ("language_code", "ja"),
             ("commit_strategy", "vad"), ("vad_silence_threshold_secs", "0.75"),
             ("no_verbatim", str(no_verbatim).lower())] + [("keyterms", t) for t in terms]
    url = "wss://api.elevenlabs.io/v1/speech-to-text/realtime?" + urlencode(query)
    t0 = time.monotonic()
    first_partial = None
    committed: list[str] = []
    async with websockets.connect(url, additional_headers={"xi-api-key": key}, max_size=None) as ws:
        chunk = SAMPLE_RATE * 2 * CHUNK_MS // 1000
        done = asyncio.Event()

        async def send():
            for i in range(0, len(pcm), chunk):
                last = i + chunk >= len(pcm)
                await ws.send(json.dumps({"message_type": "input_audio_chunk", "sample_rate": SAMPLE_RATE,
                                          "audio_base_64": base64.b64encode(pcm[i : i + chunk]).decode(),
                                          "commit": last}))
                await asyncio.sleep(CHUNK_MS / 1000)
            done.set()

        sender = asyncio.create_task(send())
        try:
            # 最後の音声を送ったあとの確定（最後の chunk で commit する）で終わり。来なければ 10 秒で諦める
            while True:
                try:
                    raw = await asyncio.wait_for(ws.recv(), timeout=10)
                except (TimeoutError, websockets.ConnectionClosed):
                    break
                e = json.loads(raw)
                kind = e.get("message_type")
                if kind == "partial_transcript" and e.get("text"):
                    first_partial = first_partial or round(time.monotonic() - t0, 2)
                elif kind == "committed_transcript":
                    if e.get("text"):
                        committed.append(e["text"])
                    if done.is_set():
                        break
                elif "error" in e:
                    raise RuntimeError(e)
        finally:
            sender.cancel()
    return {"text": "".join(committed), "segments": len(committed), "first_partial_s": first_partial,
            "total_s": round(time.monotonic() - t0, 2)}


def batch(audio: Path, key: str, terms: list[str]) -> dict:
    data: dict = {"model_id": BATCH_MODEL, "language_code": "ja", "tag_audio_events": "false"}
    if terms:
        data["keyterms"] = terms
    t0 = time.monotonic()
    with open(audio, "rb") as f:
        r = httpx.post("https://api.elevenlabs.io/v1/speech-to-text", headers={"xi-api-key": key},
                       data=data, files={"file": (audio.name, f)}, timeout=120)
    if r.is_error:
        raise RuntimeError(f"{r.status_code} {r.text[:300]}")
    return {"text": r.json()["text"], "latency_s": round(time.monotonic() - t0, 2)}


async def main() -> None:
    audio, ref = Path(sys.argv[1]), Path(sys.argv[2]).read_text().strip()
    key = os.environ["ELEVENLABS_API_KEY"]
    pcm = load_pcm(audio)
    terms = keyterms(load_keywords())
    heroes = keyterms(hero_names(), limit=50, max_len=20)
    print(f"audio {len(pcm) / SAMPLE_RATE / 2:.1f}s, keyterms batch {len(terms)} / realtime {len(heroes)}\n")
    results = {}
    runs = {
        "realtime/no-keywords": lambda: realtime(pcm, key, []),
        "realtime/keywords": lambda: realtime(pcm, key, heroes),
        "realtime/keywords+no-verbatim": lambda: realtime(pcm, key, heroes, no_verbatim=True),
        "batch/no-keywords": lambda: asyncio.to_thread(batch, audio, key, []),
        "batch/keywords": lambda: asyncio.to_thread(batch, audio, key, terms),
    }
    for name, run in runs.items():
        try:
            results[name] = await run()
        except Exception as e:
            results[name] = {"text": "", "error": str(e)[:300]}
    out = Path(__file__).parent / "results" / f"{audio.stem}.elevenlabs.json"
    out.write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n")
    for name, r in results.items():
        extra = r.get("error") or {k: v for k, v in r.items() if k != "text"}
        print(f"## {name}  CER {cer(ref, r['text']):.1%}  {extra}\n{r['text']}\n")


if __name__ == "__main__":
    asyncio.run(main())
