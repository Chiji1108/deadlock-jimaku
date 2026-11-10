# /// script
# requires-python = ">=3.12"
# dependencies = ["websockets"]
# ///
"""Qwen-Audio-3.1-ASR-Flash-Streaming（途中の字幕の候補）の日本語精度と速さを、stt_eval_openai.py の
realtime と同じように測る（100ms ずつ実時間で送る）。

usage: uv run stt_eval_qwen_streaming.py data/sample01.m4a data/sample01.txt [--repeat N]
  QWEN_SOUTH_API_KEY（シンガポールの鍵）を使う
"""

import asyncio
import json
import os
import subprocess
import sys
import time
import uuid
from pathlib import Path

import websockets

from stt_eval_openai import cer

URL = "wss://ap-southeast-1.maas.qwencloudapi.com/api-ws/v1/inference"
MODEL = "qwen-audio-3.1-asr-flash-streaming"
SAMPLE_RATE = 16000
CHUNK_MS = 100


def load_pcm(path: Path) -> bytes:
    return subprocess.run(
        ["ffmpeg", "-v", "error", "-i", str(path), "-ac", "1", "-ar", str(SAMPLE_RATE), "-f", "s16le", "-"],
        check=True,
        capture_output=True,
    ).stdout


async def stream(pcm: bytes) -> dict:
    task = uuid.uuid4().hex
    header = {"task_id": task, "streaming": "duplex"}
    t0 = time.monotonic()
    first_partial = None
    last_final = None
    sentences: dict[int, str] = {}  # sentence_id → 最新の文（途中のものも）
    partials = 0
    usage = None
    async with websockets.connect(URL, additional_headers={"Authorization": f"bearer {os.environ['QWEN_SOUTH_API_KEY']}"}) as ws:
        await ws.send(json.dumps({
            "header": {"action": "run-task", **header},
            "payload": {
                "task_group": "audio", "task": "asr", "function": "recognition", "model": MODEL,
                "parameters": {"format": "pcm", "sample_rate": SAMPLE_RATE, "language_hints": ["ja"]},
                "input": {},
            },
        }))
        started_at = sent_at = None

        async def send():
            nonlocal started_at, sent_at
            started_at = time.monotonic()
            chunk = SAMPLE_RATE * 2 * CHUNK_MS // 1000
            for i in range(0, len(pcm), chunk):
                await ws.send(pcm[i : i + chunk])
                await asyncio.sleep(CHUNK_MS / 1000)
            sent_at = time.monotonic()
            await ws.send(json.dumps({"header": {"action": "finish-task", **header}, "payload": {"input": {}}}))

        sender = None
        async for raw in ws:
            e = json.loads(raw)
            event = e["header"]["event"]
            if event == "task-failed":
                raise RuntimeError(e["header"].get("error_message"))
            if event == "task-started":
                sender = asyncio.create_task(send())
            if event == "result-generated":
                sentence = e["payload"]["output"]["sentence"]
                if sentence.get("text"):
                    first_partial = first_partial or round(time.monotonic() - started_at, 2)
                    partials += 1
                if sentence.get("sentence_end"):
                    last_final = time.monotonic()
                sentences[sentence.get("sentence_id", len(sentences))] = sentence.get("text", "")
                usage = e["payload"].get("usage") or usage
            if event == "task-finished":
                break
        await sender
    return {
        "text": "".join(sentences[k] for k in sorted(sentences)),
        "connect_s": round(started_at - t0, 2),
        "first_partial_s": first_partial,  # 音声を送り始めてから
        "last_final_after_audio_s": round(last_final - sent_at, 2) if last_final else None,
        "finished_after_audio_s": round(time.monotonic() - sent_at, 2),
        "partials": partials,
        "sentences": len(sentences),
        "usage": usage,
    }


async def main() -> None:
    args = sys.argv[1:]
    repeat = 3
    if "--repeat" in args:
        i = args.index("--repeat")
        repeat = int(args[i + 1])
        del args[i : i + 2]
    path, ref = Path(args[0]), Path(args[1]).read_text().strip()
    pcm = load_pcm(path)
    runs = []
    for _ in range(repeat):
        try:
            r = await stream(pcm)
            r["cer"] = round(cer(ref, r["text"]), 4)
        except Exception as e:
            r = {"text": "", "error": str(e)[:300]}
        runs.append(r)
        print(r.get("error") or {k: v for k, v in r.items() if k != "text"}, "\n", r["text"], "\n")
    out = Path(__file__).parent / "results" / f"{path.stem}.qwen_streaming.json"
    out.write_text(json.dumps(runs, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    asyncio.run(main())
