# /// script
# requires-python = ">=3.12"
# dependencies = ["httpx"]
# ///
"""xAI（Grok Voice Transcribe）と Qwen（Qwen3-ASR-Flash / Qwen-Audio-3.1-ASR-Flash）の確定の文字起こしを、
stt_eval_openai.py と同じ指標（文字誤り率）で測る。用語なし / ありを、それぞれ何回か流す。

usage: uv run stt_eval_xai_qwen.py data/sample01.m4a data/sample01.txt [--repeat N]
  XAI_API_KEY と QWEN_SOUTH_API_KEY（シンガポールの鍵。音声認識はシンガポールにしかない）を使う
"""

import base64
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

import httpx

XAI_MODEL = "grok-voice-transcribe-2.0"
XAI_USD_PER_SECOND = 0.10 / 3600  # REST（バッチ）は 1 時間 $0.10
QWEN = "https://ap-southeast-1.maas.qwencloudapi.com"
QWEN3_ASR = "qwen3-asr-flash"
QWEN3_ASR_USD_PER_SECOND = 0.000035
QWEN_AUDIO = "qwen-audio-3.1-asr-flash"
QWEN_AUDIO_USD_PER_MILLION = (0.15, 0.47)  # 入力・出力のトークン
XAI_KEYTERMS = 100  # 1 回に渡せる用語は 100 件・50 文字まで
QWEN_AUDIO_CONTEXT_CHARS = 400  # 文脈の文は 400 文字まで（越えた分は切られる）


def load_mp3(path: Path) -> bytes:
    return subprocess.run(
        ["ffmpeg", "-v", "error", "-i", str(path), "-ac", "1", "-ar", "16000", "-f", "mp3", "-"],
        check=True,
        capture_output=True,
    ).stdout


def load_glossary() -> dict:
    """アプリの辞書（最新の形式・版）のエンティティ"""
    root = Path.home() / "Library/Application Support/jp.deadlock.jimaku/assets"
    path = max(root.glob("v*/*/glossary.json"), key=lambda p: (int(p.parent.parent.name[1:]), int(p.parent.name)))
    return json.loads(path.read_text())["entities"]


def load_keywords() -> list[str]:
    """日本語の公式名。ヒーロー・スキル・アイテムの順（上限で切るときにヒーロー名を残す）"""
    order = {"hero": 0, "ability": 1, "item": 2}
    entities = sorted(load_glossary().values(), key=lambda e: order.get(e["kind"], 3))
    words = ["デッドロック", "ウルト"] + [e["names"]["ja"] for e in entities if e["names"].get("ja")]
    return list(dict.fromkeys(w for w in words if not re.search(r"[<>\r\n]", w)))


def xai(audio: bytes, keywords: list[str]) -> dict:
    # 選択肢はファイルより前に置く（後ろの欄は読まれないことがある）
    fields = {"model": XAI_MODEL, "language": "ja"}
    if keywords:
        fields["keyterm"] = [w for w in keywords if len(w) <= 50][:XAI_KEYTERMS]
    t0 = time.monotonic()
    r = httpx.post(
        "https://api.x.ai/v1/stt",
        headers={"Authorization": f"Bearer {os.environ['XAI_API_KEY']}"},
        data=fields,
        files={"file": ("audio.mp3", audio, "audio/mpeg")},
        timeout=120,
    )
    if r.is_error:
        raise RuntimeError(f"{r.status_code} {r.text[:300]}")
    body = r.json()
    return {
        "text": body["text"],
        "latency_s": round(time.monotonic() - t0, 2),
        "usd": body["duration"] * XAI_USD_PER_SECOND,
    }


def qwen3_asr(audio: bytes, keywords: list[str]) -> dict:
    """OpenAI 互換の口。用語は system の文脈として渡す"""
    messages = [{"role": "system", "content": [{"text": "、".join(keywords)}]}] if keywords else []
    messages.append({"role": "user", "content": [{"type": "input_audio", "input_audio": {
        "data": f"data:audio/mpeg;base64,{base64.b64encode(audio).decode()}"}}]})
    t0 = time.monotonic()
    r = httpx.post(
        f"{QWEN}/compatible-mode/v1/chat/completions",
        headers={"Authorization": f"Bearer {os.environ['QWEN_SOUTH_API_KEY']}"},
        json={"model": QWEN3_ASR, "messages": messages, "asr_options": {"language": "ja", "enable_itn": False}},
        timeout=120,
    )
    if r.is_error:
        raise RuntimeError(f"{r.status_code} {r.text[:300]}")
    body = r.json()
    seconds = body["usage"].get("seconds") or body["usage"].get("input_tokens_details", {}).get("audio_seconds", 0)
    return {
        "text": body["choices"][0]["message"]["content"],
        "latency_s": round(time.monotonic() - t0, 2),
        "usd": seconds * QWEN3_ASR_USD_PER_SECOND,
        "usage": body["usage"],
    }


def qwen_audio(audio: bytes, keywords: list[str]) -> dict:
    """Qwen 独自の口（OpenAI 互換の口では使えない）。用語は前の発話の文として渡す（400 文字まで）"""
    content = []
    if keywords:
        text = ""
        for w in keywords:
            if len(text) + len(w) + 1 > QWEN_AUDIO_CONTEXT_CHARS:
                break
            text += w + "、"
        content.append({"type": "input_text", "text": text})
    content.append({"type": "input_audio", "input_audio": {
        "data": f"data:audio/mpeg;base64,{base64.b64encode(audio).decode()}"}})
    t0 = time.monotonic()
    r = httpx.post(
        f"{QWEN}/api/v1/services/aigc/multimodal-generation/generation",
        headers={"Authorization": f"Bearer {os.environ['QWEN_SOUTH_API_KEY']}", "X-DashScope-SSE": "disable"},
        json={
            "model": QWEN_AUDIO,
            "input": {"messages": [{"role": "user", "content": content}]},
            "parameters": {"format": "mp3", "sample_rate": "16000", "language_hints": ["ja"]},
        },
        timeout=120,
    )
    if r.is_error:
        raise RuntimeError(f"{r.status_code} {r.text[:300]}")
    body = r.json()
    output = body.get("output", body)
    usage = body.get("usage", {})
    inp, out = QWEN_AUDIO_USD_PER_MILLION
    usd = (usage.get("input_tokens", 0) * inp + usage.get("output_tokens", 0) * out) / 1e6
    return {
        "text": output.get("text") or output["sentence"]["text"],
        "latency_s": round(time.monotonic() - t0, 2),
        "usd": usd,
        "usage": usage,
    }


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


def main() -> None:
    args = sys.argv[1:]
    repeat = 3
    if "--repeat" in args:
        i = args.index("--repeat")
        repeat = int(args[i + 1])
        del args[i : i + 2]
    path, ref = Path(args[0]), Path(args[1]).read_text().strip()
    audio = load_mp3(path)
    keywords = load_keywords()
    print(f"keywords {len(keywords)}\n")
    results: dict[str, list] = {}
    for engine, run in [("xai", xai), ("qwen3-asr-flash", qwen3_asr), ("qwen-audio-3.1-asr-flash", qwen_audio)]:
        for name, kw in [("no-keywords", []), ("keywords", keywords)]:
            runs = results.setdefault(f"{engine}/{name}", [])
            for _ in range(repeat):
                try:
                    r = run(audio, kw)
                    r["cer"] = round(cer(ref, r["text"]), 4)
                except Exception as e:
                    r = {"text": "", "error": str(e)[:300]}
                runs.append(r)
    out = Path(__file__).parent / "results" / f"{path.stem}.xai_qwen.json"
    out.write_text(json.dumps(results, ensure_ascii=False, indent=2))
    for name, runs in results.items():
        print(f"## {name}")
        for r in runs:
            extra = r.get("error") or f"CER {r['cer']:.1%}  {r['latency_s']}s  ${r['usd']:.5f}"
            print(f"  {extra}\n  {r['text']}")
        print()


if __name__ == "__main__":
    main()
