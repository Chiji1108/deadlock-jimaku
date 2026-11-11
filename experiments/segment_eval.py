# /// script
# requires-python = ">=3.12"
# dependencies = ["websockets"]
# ///
"""発話の区切りを、会社に任せた場合とアプリの VAD で比べる（DESIGN.md 3.5）。
録音を実時間で流し、会社が区切った文と、区切りが届くまでの時間（話し終えてから）を見る。

usage: uv run segment_eval.py data/sample01.wav
  Qwen-Audio-3.1-ASR-Flash-Streaming（QWEN_SOUTH_API_KEY。サーバーが文を区切る）と、
  GPT-Live-Transcribe の `turn_detection`（OPENAI_API_KEY。server_vad と semantic_vad）を測る。
  アプリの VAD の区切りは `cargo run -p overlays-core --example segments -- AUDIO.wav 300 500 750`、
  その区切りを文字に直すのには、Qwen が返す文字ごとの時刻を使う
  出力: results/NAME.segments.json
"""

import asyncio
import base64
import json
import os
import subprocess
import sys
import time
import uuid
from pathlib import Path

import websockets

CHUNK_MS = 100


def pcm(path: Path, rate: int) -> bytes:
    cmd = ["ffmpeg", "-v", "error", "-i", str(path), "-ac", "1", "-ar", str(rate), "-f", "s16le", "-"]
    return subprocess.run(cmd, check=True, capture_output=True).stdout


async def qwen(path: Path) -> dict:
    audio = pcm(path, 16000)
    task = uuid.uuid4().hex
    header = {"task_id": task, "streaming": "duplex"}
    url = "wss://ap-southeast-1.maas.qwencloudapi.com/api-ws/v1/inference"
    key = os.environ["QWEN_SOUTH_API_KEY"]
    sentences: dict[int, dict] = {}
    async with websockets.connect(url, additional_headers={"Authorization": f"bearer {key}"}) as ws:
        await ws.send(json.dumps({
            "header": {"action": "run-task", **header},
            "payload": {
                "task_group": "audio", "task": "asr", "function": "recognition",
                "model": "qwen-audio-3.1-asr-flash-streaming",
                "parameters": {"format": "pcm", "sample_rate": 16000, "language_hints": ["ja"]},
                "input": {},
            },
        }))
        started = None

        async def send():
            nonlocal started
            started = time.monotonic()
            step = 16000 * 2 * CHUNK_MS // 1000
            for i in range(0, len(audio), step):
                await ws.send(audio[i : i + step])
                await asyncio.sleep(CHUNK_MS / 1000)
            # 最後の文を閉じさせる無音
            for _ in range(20):
                await ws.send(bytes(step))
                await asyncio.sleep(CHUNK_MS / 1000)
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
                s = e["payload"]["output"]["sentence"]
                if s.get("sentence_end"):
                    sentences[s["sentence_id"]] = {
                        "start": s["begin_time"], "end": s["end_time"], "text": s["text"],
                        "arrivedMs": round((time.monotonic() - started) * 1000),
                        "words": [[w["begin_time"], w["text"] + (w.get("punctuation") or "")] for w in s["words"]],
                    }
            if event == "task-finished":
                break
        await sender
    return {"segments": [sentences[k] for k in sorted(sentences)]}


async def openai(path: Path, detection: dict) -> dict:
    audio = pcm(path, 24000)
    url = "wss://api.openai.com/v1/realtime?intent=transcription"
    key = os.environ["OPENAI_API_KEY"]
    turns: dict[str, dict] = {}
    async with websockets.connect(url, additional_headers={"Authorization": f"Bearer {key}"}) as ws:
        async def expect(kind: str):
            async for raw in ws:
                e = json.loads(raw)
                if e["type"] == "error":
                    raise RuntimeError(e["error"])
                if e["type"] == kind:
                    return

        await expect("session.created")
        await ws.send(json.dumps({"type": "session.update", "session": {
            "type": "transcription",
            "audio": {"input": {
                "format": {"type": "audio/pcm", "rate": 24000},
                "transcription": {"model": "gpt-live-transcribe", "languages": ["ja"], "delay": "low"},
                "turn_detection": detection,
            }},
        }}))
        await expect("session.updated")
        started = time.monotonic()

        async def send():
            step = 24000 * 2 * CHUNK_MS // 1000
            silence = bytes(step)
            for i in range(0, len(audio) + step * 30, step):
                chunk = audio[i : i + step] or silence
                await ws.send(json.dumps({"type": "input_audio_buffer.append", "audio": base64.b64encode(chunk).decode()}))
                await asyncio.sleep(CHUNK_MS / 1000)

        sender = asyncio.create_task(send())
        try:
            while True:
                raw = await asyncio.wait_for(ws.recv(), timeout=8 if sender.done() else 60)
                e = json.loads(raw)
                now = round((time.monotonic() - started) * 1000)
                item = e.get("item_id")
                if e["type"] == "input_audio_buffer.speech_started":
                    turns[item] = {"start": e["audio_start_ms"]}
                elif e["type"] == "input_audio_buffer.speech_stopped":
                    turns.setdefault(item, {}).update(end=e["audio_end_ms"], arrivedMs=now)
                elif e["type"] == "conversation.item.input_audio_transcription.completed":
                    turns.setdefault(item, {}).update(text=e["transcript"], textMs=now)
                elif e["type"] == "error":
                    raise RuntimeError(e["error"])
        except TimeoutError:
            pass
        sender.cancel()
    return {"segments": list(turns.values())}


async def main() -> None:
    path = Path(sys.argv[1])
    out = Path(__file__).parent / "results" / f"{path.stem}.segments.json"
    report = json.loads(out.read_text()) if out.exists() else {}
    runs = {
        "qwen-streaming": lambda: qwen(path),
        "openai-server_vad": lambda: openai(path, {"type": "server_vad"}),
        "openai-semantic_vad": lambda: openai(path, {"type": "semantic_vad"}),
    }
    for name, run in runs.items():
        try:
            report[name] = await run()
        except Exception as e:
            report[name] = {"error": str(e)[:300]}
        segments = report[name].get("segments", [])
        print(f"## {name}: {report[name].get('error') or len(segments)}")
        for s in segments:
            late = s.get("arrivedMs", 0) - s.get("end", 0)
            print(f"  {s.get('start')}–{s.get('end')} (+{late} ms) {s.get('text')}")
    out.write_text(json.dumps(report, ensure_ascii=False, indent=1) + "\n")


if __name__ == "__main__":
    asyncio.run(main())
