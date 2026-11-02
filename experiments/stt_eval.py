# /// script
# requires-python = ">=3.12"
# dependencies = ["httpx", "websockets"]
# ///
"""Voxtral 3 の日本語精度を 4 条件（realtime / batch × context_bias なし / あり）で比べる。

usage: uv run stt_eval.py data/sample01.m4a data/sample01.txt
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

API = "https://api.mistral.ai/v1"
DEADLOCK_API = "https://api.deadlock-api.com/v1/assets"
REALTIME_MODEL = "voxtral-mini-transcribe-realtime-3"
BATCH_MODEL = "voxtral-small-transcribe-3"
SAMPLE_RATE = 16000
CHUNK_MS = 100
STREAMING_DELAY_MS = 480
KEY_TERMS = ["デッドロック", "ヘイズ", "バイオレット", "バレットダンス", "ウルト"]


def load_pcm(path: Path) -> bytes:
    return subprocess.run(
        ["ffmpeg", "-v", "error", "-i", str(path), "-ac", "1", "-ar", str(SAMPLE_RATE), "-f", "s16le", "-"],
        check=True,
        capture_output=True,
    ).stdout


def load_glossary() -> list[str]:
    """ヒーロー（player_selectable かつ disabled でないもの。in_development は見ない）・ショップのアイテム・ヒーローのスキルの日本語名。"""
    with httpx.Client(timeout=60) as c:
        heroes = c.get(f"{DEADLOCK_API}/heroes", params={"language": "japanese"}).json()
        items = c.get(f"{DEADLOCK_API}/items", params={"language": "japanese"}).json()
    heroes = [h for h in heroes if h["player_selectable"] and not h["disabled"]]
    by_class = {i["class_name"]: i for i in items}
    abilities = [
        by_class[cls]["name"]
        for h in heroes
        for slot, cls in h["items"].items()
        if slot.startswith("signature") and cls in by_class
    ]
    shop = [i["name"] for i in items if i.get("type") == "upgrade" and i.get("shopable")]
    print(f"glossary: heroes {len(heroes)}, items {len(shop)}, abilities {len(abilities)}")
    names = [h["name"] for h in heroes] + shop + abilities + ["デッドロック", "ウルト"]
    return list(dict.fromkeys(n for n in names if n and not n.startswith(("citadel_", "ability_"))))


async def realtime(pcm: bytes, key: str, context_bias: list[str]) -> dict:
    url = f"wss://api.mistral.ai/v1/audio/transcriptions/realtime?model={REALTIME_MODEL}"
    session = {
        "audio_format": {"encoding": "pcm_s16le", "sample_rate": SAMPLE_RATE},
        "target_streaming_delay_ms": STREAMING_DELAY_MS,
        "language": "ja",
        "context_bias": context_bias,
        "turn_detection": {},
    }
    events: list[dict] = []
    async with websockets.connect(url, additional_headers={"Authorization": f"Bearer {key}"}, max_size=None) as ws:
        first = json.loads(await ws.recv())
        if first["type"] != "session.created":
            raise RuntimeError(first)
        await ws.send(json.dumps({"type": "session.update", "session": session}))
        while (msg := json.loads(await ws.recv()))["type"] != "session.updated":
            if msg["type"] == "error":
                raise RuntimeError(msg)

        start = time.monotonic()
        chunk = SAMPLE_RATE * 2 * CHUNK_MS // 1000

        async def send() -> None:
            # 実時間と同じ速さで送る（遅延を本番に近い条件で測るため）
            for i, off in enumerate(range(0, len(pcm), chunk)):
                await asyncio.sleep(max(0, start + i * CHUNK_MS / 1000 - time.monotonic()))
                await ws.send(json.dumps({"type": "input_audio.append", "audio": base64.b64encode(pcm[off : off + chunk]).decode()}))
            await ws.send(json.dumps({"type": "input_audio.end"}))

        sender = asyncio.create_task(send())
        done = None
        async for raw in ws:
            msg = json.loads(raw)
            msg["_t"] = round(time.monotonic() - start, 3)
            events.append(msg)
            if msg["type"] == "error":
                raise RuntimeError(msg)
            if msg["type"] == "transcription.done":
                done = msg
                break
        await sender

    return {"events": events, "text": done["text"] if done else "", "turns": split_turns(events)}


def split_turns(events: list[dict]) -> list[dict]:
    """turn.start で開き、turn.end 後も streaming delay の間は delta を受け付けて閉じる（DESIGN.md 3.1）。"""
    turns: list[dict] = []
    cur = None
    for e in events:
        if cur and cur.get("closeAt") is not None and (e["_t"] > cur["closeAt"] or e["type"] == "turn.start"):
            turns.append(cur)
            cur = None
        if e["type"] == "turn.start":
            cur = {"start_s": e.get("time_s"), "firstDeltaAt": None, "text": ""}
        elif e["type"] == "transcription.text.delta":
            if cur is None:
                cur = {"start_s": None, "firstDeltaAt": None, "text": ""}
            cur["firstDeltaAt"] = cur["firstDeltaAt"] or e["_t"]
            cur["text"] += e["text"]
        elif e["type"] == "turn.end" and cur:
            cur["end_s"] = e.get("time_s")
            cur["endReason"] = e.get("end_reason")
            cur["closeAt"] = e["_t"] + STREAMING_DELAY_MS / 1000
    if cur:
        turns.append(cur)
    return turns


def batch(audio: Path, key: str, context_bias: list[str]) -> dict:
    data = {"model": BATCH_MODEL, "language": "ja", "context_bias": context_bias}
    t0 = time.monotonic()
    with open(audio, "rb") as f:
        r = httpx.post(
            f"{API}/audio/transcriptions",
            headers={"Authorization": f"Bearer {key}"},
            data=data,
            files={"file": (audio.name, f)},
            timeout=120,
        )
    if r.is_error:
        raise RuntimeError(f"{r.status_code} {r.text}")
    return {"text": r.json()["text"], "latency_s": round(time.monotonic() - t0, 2), "raw": r.json()}


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


def term_hits(ref: str, hyp: str) -> str:
    return " ".join(f"{t}{'○' if t in hyp else '×'}" for t in KEY_TERMS if t in ref)


async def main() -> None:
    audio, ref_path = Path(sys.argv[1]), Path(sys.argv[2])
    ref = ref_path.read_text().strip()
    key = os.environ["MISTRAL_API_KEY"]
    pcm = load_pcm(audio)
    glossary = load_glossary()
    print(f"audio {len(pcm) / SAMPLE_RATE / 2:.1f}s, context_bias {len(glossary)} terms\n")

    results = {}
    for bias_name, bias in [("no-bias", []), ("bias", glossary)]:
        results[f"realtime/{bias_name}"] = await realtime(pcm, key, bias)
        results[f"batch/{bias_name}"] = batch(audio, key, bias)

    out = Path(__file__).parent / "results" / f"{audio.stem}.json"
    out.parent.mkdir(exist_ok=True)
    out.write_text(json.dumps(results, ensure_ascii=False, indent=2))

    print(f"{'condition':<18} {'CER':>6}  terms")
    for name, r in results.items():
        print(f"{name:<18} {cer(ref, r['text']):>6.1%}  {term_hits(ref, r['text'])}")
    for name, r in results.items():
        print(f"\n## {name}" + (f"  (latency {r['latency_s']}s)" if "latency_s" in r else ""))
        print(r["text"])
        for t in r.get("turns", []):
            print(f"  turn {t.get('start_s')}–{t.get('end_s')}s ({t.get('endReason')}) firstDelta@{t['firstDeltaAt']}s: {t['text']}")
    print(f"\nraw: {out}")


asyncio.run(main())
