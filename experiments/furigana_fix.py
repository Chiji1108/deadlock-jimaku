# /// script
# requires-python = ">=3.12"
# dependencies = ["httpx"]
# ///
"""辞書（Lindera）の読みを小さい LLM に直させて、直せた文と壊した文を数える（DESIGN.md 9.3）。
Chrome の Gemini Nano（`chrome_eval.mjs furigana`）と同じ文、同じ問い、同じ数え方で、OpenRouter のモデルを測る。

usage: OPENROUTER_API_KEY=... uv run furigana_fix.py REPEAT MODEL...
  出力: results/furigana_fix.json（モデルごと。前の結果は残す）
"""

import json
import os
import re
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import httpx

HERE = Path(__file__).parent
OUT = HERE / "results" / "furigana_fix.json"
# chrome_eval.mjs の FURIGANA_SYSTEM と同じ
SYSTEM = (
    "You check the readings (furigana) of a Japanese subtitle from a video game streamer. "
    'The input is JSON: "sentence" is the subtitle, and "words" lists its words that contain kanji, each with the reading a dictionary gave, in hiragana. '
    "The dictionary reads each word without looking at the sentence, so some readings are wrong for this sentence (for example 辛い as つらい when it means spicy, or 一 and 人 as いち and にん when the sentence says ひとり). "
    'Answer with JSON {"readings": [...]}: the correct hiragana reading of each word, in the same order and the same number as "words". '
    "Keep the dictionary's reading unless it is wrong in this sentence. A reading covers only the characters of its word."
)
KANA = re.compile(r"^[ぁ-ゖー]+$")
http = httpx.Client(timeout=60, headers={"Authorization": f"Bearer {os.environ['OPENROUTER_API_KEY']}"})


def right(words, readings):
    return all(r in w.get("accept", [w["reading"]]) for w, r in zip(words, readings))


def ask(model, case):
    words = [w for w in case["words"] if w["reading"]]
    schema = {
        "type": "object", "required": ["readings"], "additionalProperties": False,
        "properties": {"readings": {"type": "array", "items": {"type": "string"}}},
    }
    sent = {"sentence": case["text"], "words": [{"text": w["text"], "reading": w["reading"]} for w in words]}
    body = {
        "model": model,
        "messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": json.dumps(sent, ensure_ascii=False)}],
        "response_format": {"type": "json_schema", "json_schema": {"name": "readings", "strict": True, "schema": schema}},
    }
    dictionary = right(words, [w["reading"] for w in words])
    t0 = time.monotonic()
    readings, cost, error = [], 0.0, None
    try:
        res = http.post("https://openrouter.ai/api/v1/chat/completions", json=body)
        res.raise_for_status()
        data = res.json()
        cost = data.get("usage", {}).get("cost") or 0.0
        readings = json.loads(data["choices"][0]["message"]["content"])["readings"]
    except Exception as e:  # noqa: BLE001 形が合わない答えも、届かない答えも「使えない」と数える
        error = repr(e)[:200]
    ms = round((time.monotonic() - t0) * 1000)
    # 形が合わない答えは使えない（アプリなら辞書の読みに戻す）
    usable = len(readings) == len(words) and all(isinstance(r, str) and KANA.match(r) for r in readings)
    nano = right(words, readings) if usable else dictionary
    changed = [f"{w['text']}: {w['reading']} → {r}" for w, r in zip(words, readings) if r != w["reading"]] if usable else []
    return {"text": case["text"], "ms": ms, "usable": usable, "dictionary": dictionary, "fixed": nano, "changed": changed, "cost": cost, "error": error}


def main():
    repeat, models = int(sys.argv[1]), sys.argv[2:]
    cases = json.loads((HERE / "data" / "furigana_cases.json").read_text())
    report = json.loads(OUT.read_text()) if OUT.exists() else {}
    for model in models:
        with ThreadPoolExecutor(4) as pool:
            runs = list(pool.map(lambda c: ask(model, c), cases * repeat))
        times = sorted(r["ms"] for r in runs)
        summary = {
            "runs": len(runs),
            "dictionaryRight": sum(r["dictionary"] for r in runs),
            "right": sum(r["fixed"] for r in runs),
            "fixed": sum(not r["dictionary"] and r["fixed"] for r in runs),
            "broken": sum(r["dictionary"] and not r["fixed"] for r in runs),
            "stillWrong": sum(not r["dictionary"] and not r["fixed"] for r in runs),
            "unusable": sum(not r["usable"] for r in runs),
            "medianMs": times[len(times) // 2],
            "p90Ms": times[len(times) * 9 // 10],
            "usd": round(sum(r["cost"] for r in runs), 5),
        }
        report[model] = {"summary": summary, "runs": runs}
        print(model, json.dumps(summary), flush=True)
        errors = {r["error"] for r in runs if r["error"]}
        if errors:
            print("  errors:", list(errors)[:2], flush=True)
    OUT.write_text(json.dumps({"system": SYSTEM, **report}, ensure_ascii=False, indent=1) + "\n")


main()
