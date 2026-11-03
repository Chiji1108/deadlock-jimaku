# /// script
# requires-python = ">=3.12"
# dependencies = ["httpx", "websockets"]
# ///
"""OpenAI の音声認識（gpt-live-transcribe / gpt-transcribe）の日本語精度を、stt_eval.py（Voxtral）と
同じ指標で測る。用語は keywords で渡す。

usage: uv run stt_eval_openai.py data/sample01.m4a data/sample01.txt
"""

import asyncio
import base64
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

import httpx
import websockets

REALTIME_MODEL = "gpt-live-transcribe"
BATCH_MODEL = "gpt-transcribe"
SAMPLE_RATE = 24000  # Realtime の audio/pcm は 24kHz
CHUNK_MS = 100
PROMPT = "Live commentary by a Japanese streamer playing the video game Deadlock (Valve)."


def load_pcm(path: Path) -> bytes:
    return subprocess.run(
        ["ffmpeg", "-v", "error", "-i", str(path), "-ac", "1", "-ar", str(SAMPLE_RATE), "-f", "s16le", "-"],
        check=True,
        capture_output=True,
    ).stdout


def load_glossary() -> dict:
    """アプリの辞書（最新の形式・版）のエンティティ"""
    root = Path.home() / "Library/Application Support/dev.deadlock-stream-overlays.app/assets"
    path = max(root.glob("v*/*/glossary.json"), key=lambda p: (int(p.parent.parent.name[1:]), int(p.parent.name)))
    return json.loads(path.read_text())["entities"]


def load_keywords() -> list[str]:
    """アプリの辞書から、日本語の公式名と別名"""
    entities = load_glossary()
    aliases = json.loads((Path(__file__).parent.parent / "data" / "aliases.ja.json").read_text())
    words = [e["names"]["ja"] for e in entities.values() if e["names"].get("ja")]
    words += [a for xs in aliases.values() for a in xs]
    return list(dict.fromkeys(w for w in words if not re.search(r"[<>\r\n]", w)))


async def realtime(pcm: bytes, key: str, keywords: list[str]) -> dict:
    url = "wss://api.openai.com/v1/realtime?intent=transcription"
    t0 = time.monotonic()
    first_delta = None
    text = ""
    async with websockets.connect(url, additional_headers={"Authorization": f"Bearer {key}"}) as ws:
        transcription = {"model": REALTIME_MODEL, "languages": ["ja"], "delay": "low", "prompt": PROMPT}
        if keywords:
            transcription["keywords"] = keywords
        await ws.send(json.dumps({
            "type": "session.update",
            "session": {"type": "transcription", "audio": {"input": {
                "format": {"type": "audio/pcm", "rate": SAMPLE_RATE},
                "transcription": transcription,
                "turn_detection": None,
            }}},
        }))
        chunk = SAMPLE_RATE * 2 * CHUNK_MS // 1000

        async def send():
            for i in range(0, len(pcm), chunk):
                await ws.send(json.dumps({"type": "input_audio_buffer.append",
                                          "audio": base64.b64encode(pcm[i : i + chunk]).decode()}))
                await asyncio.sleep(CHUNK_MS / 1000)
            await ws.send(json.dumps({"type": "input_audio_buffer.commit"}))

        sender = asyncio.create_task(send())
        async for raw in ws:
            e = json.loads(raw)
            if e["type"] == "error":
                raise RuntimeError(e)
            if e["type"] == "conversation.item.input_audio_transcription.delta":
                first_delta = first_delta or round(time.monotonic() - t0, 2)
                text += e.get("delta", "")
            if e["type"] == "conversation.item.input_audio_transcription.completed":
                text = e.get("transcript", text)
                break
        await sender
    return {"text": text, "first_delta_s": first_delta, "total_s": round(time.monotonic() - t0, 2)}


def batch(audio: Path, key: str, keywords: list[str]) -> dict:
    data: dict = {"model": BATCH_MODEL, "prompt": PROMPT, "languages[]": ["ja"]}
    if keywords:
        data["keywords[]"] = keywords
    t0 = time.monotonic()
    with open(audio, "rb") as f:
        r = httpx.post(
            "https://api.openai.com/v1/audio/transcriptions",
            headers={"Authorization": f"Bearer {key}"},
            data=data,
            files={"file": (audio.name, f)},
            timeout=120,
        )
    if r.is_error:
        raise RuntimeError(f"{r.status_code} {r.text[:300]}")
    return {"text": r.json()["text"], "latency_s": round(time.monotonic() - t0, 2)}


def normalize(s: str) -> str:
    return re.sub(r"[\s、。，．,.!?！？「」…〜]", "", s)


def cer(ref: str, hyp: str) -> float:
    a, b = normalize(ref), normalize(hyp)
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (ca != cb)))
        prev = cur
    return prev[-1] / max(len(a), 1)


async def main() -> None:
    audio, ref = Path(sys.argv[1]), Path(sys.argv[2]).read_text().strip()
    key = os.environ["OPENAI_API_KEY"]
    pcm = load_pcm(audio)
    keywords = load_keywords()
    print(f"audio {len(pcm) / SAMPLE_RATE / 2:.1f}s, keywords {len(keywords)}\n")
    results = {}
    for name, kw in [("no-keywords", []), ("keywords", keywords)]:
        for kind, run in [("realtime", lambda: realtime(pcm, key, kw)), ("batch", lambda: asyncio.to_thread(batch, audio, key, kw))]:
            try:
                results[f"{kind}/{name}"] = await run()
            except Exception as e:
                results[f"{kind}/{name}"] = {"text": "", "error": str(e)[:300]}
    out = Path(__file__).parent / "results" / f"{audio.stem}.openai.json"
    out.write_text(json.dumps(results, ensure_ascii=False, indent=2))
    for name, r in results.items():
        extra = r.get("error") or {k: v for k, v in r.items() if k != "text"}
        print(f"## {name}  CER {cer(ref, r['text']):.1%}  {extra}\n{r['text']}\n")


if __name__ == "__main__":
    asyncio.run(main())
