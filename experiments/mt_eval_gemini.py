"""Google の Gemini（Flash、Flash-Lite）に翻訳だけを頼んで、用語の名前が崩れないかを測る（DESIGN.md 9.7）。
文と数え方は `split_eval translate` と同じ: 正解の用語を `⟦英語名⟧` に置き換えた原文（results/split_eval.terms_cases.json
の `source`）を英語とロシア語に訳させ、名前が抜けた・知らない名前を書いた・語尾を付けた・日本語が残ったものを崩れと数える。

usage: python3 mt_eval_gemini.py MODEL... [--repeat N]
  GEMINI_API_KEY を使う。結果は results/split_eval.terms_cases.json に `translate/MODEL` として足す
"""

import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).parent
RESULTS = HERE / "results/split_eval.terms_cases.json"
TARGETS = {"en": "English", "ru": "Russian"}
DOMAIN = (
    "Casual live commentary by a streamer playing the video game Deadlock, like gamers talk. "
    "Words in ⟦ ⟧ are game terms (names of heroes, items and abilities); "
    "keep them exactly as they are, with the brackets."
)


def problems(translation: str, names: list[str]) -> list[str]:
    found, out = [], []
    for match in re.finditer(r"⟦([^⟦⟧]*)⟧(.?)", translation):
        name, after = match.group(1), match.group(2)
        if name not in names:
            out.append(f"unknown: {name}")
        if after.isalpha():
            out.append(f"inflected: {name}")
        found.append(name)
    if translation.count("⟦") != translation.count("⟧"):
        out.append("unclosed")
    out += [f"missing: {name}" for name in names if name not in found]
    if re.search(r"[぀-ヿ一-鿿]", translation):
        out.append("japanese")
    return out


def translate(model: str, source: str, language: str) -> dict:
    body = {
        "systemInstruction": {"parts": [{"text": f"Translate the Japanese text into {language}. {DOMAIN} Reply with the translation only."}]},
        "contents": [{"role": "user", "parts": [{"text": source}]}],
        "generationConfig": {"temperature": 0},
    }
    request = urllib.request.Request(
        f"https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent",
        data=json.dumps(body).encode(),
        headers={"x-goog-api-key": os.environ["GEMINI_API_KEY"], "Content-Type": "application/json"},
    )
    for attempt in range(5):
        started = time.monotonic()
        try:
            with urllib.request.urlopen(request, timeout=60) as res:
                answer = json.loads(res.read())
            parts = answer["candidates"][0].get("content", {}).get("parts", [])
            return {"text": "".join(p.get("text", "") for p in parts).strip(), "ms": round((time.monotonic() - started) * 1000), "usage": answer.get("usageMetadata", {})}
        except urllib.error.HTTPError as e:
            detail = e.read().decode()[:200]
            print(f"  {e.code} {detail}", file=sys.stderr)
            if e.code == 400:
                raise SystemExit(detail)
            time.sleep(5 * (attempt + 1))
        except TimeoutError:
            print("  timed out", file=sys.stderr)
    return {"text": "", "ms": 0, "usage": {}}


def main() -> None:
    args = sys.argv[1:]
    repeat = int(args[args.index("--repeat") + 1]) if "--repeat" in args else 3
    models = [a for a in args if not a.startswith("--") and not a.isdigit()]
    results = json.loads(RESULTS.read_text())
    # 同じ文が 3 回ずつ並んでいるので、1 つずつ取る
    cases = results["translate/qwen-mt-flash"]["runs"][::3]
    for model in models:
        runs, tokens = [], [0, 0]
        for case in cases:
            names = re.findall(r"⟦([^⟦⟧]*)⟧", case["source"])
            for _ in range(repeat):
                got = {lang: translate(model, case["source"], name) for lang, name in TARGETS.items()}
                found = {lang: problems(g["text"], names) for lang, g in got.items()}
                for g in got.values():
                    tokens[0] += g["usage"].get("promptTokenCount", 0)
                    tokens[1] += g["usage"].get("candidatesTokenCount", 0)
                runs.append({
                    "text": case["text"],
                    "source": case["source"],
                    "ms": max(g["ms"] for g in got.values()),
                    "ok": not any(found.values()),
                    "translations": {lang: g["text"] for lang, g in got.items()},
                    "problems": found,
                })
                time.sleep(0.5)
        times = sorted(r["ms"] for r in runs)
        summary = {
            "runs": len(runs),
            "ok": sum(r["ok"] for r in runs),
            "en": sum(not r["problems"]["en"] for r in runs),
            "ru": sum(not r["problems"]["ru"] for r in runs),
            "medianMs": times[len(times) // 2],
            "p90Ms": times[len(times) * 9 // 10],
            "inputTokens": tokens[0],
            "outputTokens": tokens[1],
        }
        print(f"translate/{model}: {json.dumps(summary)}", flush=True)
        results = json.loads(RESULTS.read_text())
        results[f"translate/{model}"] = {"summary": summary, "runs": runs}
        RESULTS.write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
