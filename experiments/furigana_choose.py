# /// script
# requires-python = ">=3.12"
# dependencies = ["httpx"]
# ///
"""辞書（Lindera）の上位の分け方から出た読みの候補を、判定モデルに選ばせる（DESIGN.md 6.4）。
候補は `cargo run -p overlays-core --example nbest_cover -- data/furigana_cases.json 20 2> CANDIDATES.jsonl` で作る。

usage: OPENROUTER_API_KEY=... uv run furigana_choose.py CANDIDATES.jsonl REPEAT MODEL...
  出力: results/furigana_choose.json
"""

import json
import os
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import httpx

OUT = Path(__file__).parent / "results" / "furigana_choose.json"
INSTRUCTIONS = (
    "The state is a Japanese subtitle from a video game streamer. Each choice is a way to read the whole subtitle aloud, in hiragana. "
    "Choose the reading that is correct for this sentence in context."
)
http = httpx.Client(timeout=30, headers={"Authorization": f"Bearer {os.environ['OPENROUTER_API_KEY']}"})


def ask(model, case):
    candidates = case["candidates"]
    right = lambda reading: reading in case["gold"]  # noqa: E731
    dictionary = right(case["dictionary"])
    if len(candidates) < 2:
        return {"text": case["text"], "ms": 0, "asked": False, "dictionary": dictionary, "right": dictionary, "cost": 0.0}
    criteria = {f"r{i}": reading for i, reading in enumerate(candidates)}
    body = {"model": model, "state": case["text"], "questions": {"reading": {"type": "choice", "instructions": INSTRUCTIONS, "criteria": criteria}}}
    t0 = time.monotonic()
    chosen, cost = case["dictionary"], 0.0
    try:
        res = http.post("https://openrouter.ai/api/v1/systemone", json=body)
        res.raise_for_status()
        data = res.json()
        cost = data.get("usage", {}).get("cost") or 0.0
        chosen = criteria.get(data["answers"]["reading"]["choice"], chosen)
    except Exception as e:  # noqa: BLE001 聞けなければ辞書の読みのまま
        print("  error:", repr(e)[:160], flush=True)
    ms = round((time.monotonic() - t0) * 1000)
    return {"text": case["text"], "ms": ms, "asked": True, "dictionary": dictionary, "right": right(chosen), "chosen": chosen, "cost": cost}


def main():
    cases = [json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines() if line.startswith("{")]
    repeat, models = int(sys.argv[2]), sys.argv[3:]
    report = json.loads(OUT.read_text()) if OUT.exists() else {}
    for model in models:
        with ThreadPoolExecutor(4) as pool:
            runs = list(pool.map(lambda c: ask(model, c), cases * repeat))
        times = sorted(r["ms"] for r in runs if r["asked"])
        summary = {
            "runs": len(runs),
            "dictionaryRight": sum(r["dictionary"] for r in runs),
            "right": sum(r["right"] for r in runs),
            "fixed": sum(not r["dictionary"] and r["right"] for r in runs),
            "broken": sum(r["dictionary"] and not r["right"] for r in runs),
            "asked": len(times),
            "medianMs": times[len(times) // 2],
            "p90Ms": times[len(times) * 9 // 10],
            "usd": round(sum(r["cost"] for r in runs), 5),
        }
        report[model] = {"summary": summary, "runs": runs}
        print(model, json.dumps(summary), flush=True)
    OUT.write_text(json.dumps(report, ensure_ascii=False, indent=1) + "\n")


main()
