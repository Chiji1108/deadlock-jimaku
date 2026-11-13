"""PC の中で動く LLM（llama.cpp の llama-server）に翻訳だけを頼んで、用語の名前が崩れないかを測る（DESIGN.md 9.7）。
文と数え方は `split_eval translate`・`mt_eval_gemini.py` と同じ（63 文、英語とロシア語）。

usage: python3 mt_eval_llama.py LABEL [--grammar] [--repeat N] [--url LLAMA_URL]
  LABEL は結果に付ける名前（例: gemma-4-e4b-q4）。結果は results/split_eval.terms_cases.json に `translate/local-LABEL` として足す。
  --grammar を付けると、llama.cpp の文法で「⟦ のあとは、その文の用語の名前しか書けない」と縛る（結果の名前に `+grammar` が付く）
"""

import json
import re
import sys
import time
import urllib.request

from mt_eval_gemini import DOMAIN, RESULTS, TARGETS, problems


def grammar(names: list[str]) -> str:
    """⟦…⟧ の中は、その文の用語の名前だけ。用語のない文では、括弧を書けない。
    括弧の直後に字を続けられない（`⟦Sticky Bomb⟧s` のように語尾を付けさせない。`'s` は書ける）"""
    if not names:
        return "root ::= [^⟦⟧]+\n"
    quoted = " | ".join(json.dumps(name, ensure_ascii=False) for name in dict.fromkeys(names))
    return (
        "root ::= text? (term after?)+\n"
        f'term ::= "⟦" ({quoted}) "⟧"\n'
        "after ::= [^⟦⟧a-zA-Zа-яёА-ЯЁ] [^⟦⟧]*\n"
        "text ::= [^⟦⟧]+\n"
    )


def translate(url: str, source: str, language: str, names: list[str], constrain: bool) -> dict:
    body = {
        "messages": [
            {"role": "system", "content": f"Translate the Japanese text into {language}. {DOMAIN} Reply with the translation only."},
            {"role": "user", "content": source},
        ],
        "temperature": 0,
        "max_tokens": 200,
        "cache_prompt": True,
        # 答える前に考えさせない（考えを本文に書くモデルがある。字幕には遅すぎる）
        "chat_template_kwargs": {"enable_thinking": False},
    }
    if constrain:
        body["grammar"] = grammar(names)
    request = urllib.request.Request(
        f"{url}/v1/chat/completions", data=json.dumps(body).encode(), headers={"Content-Type": "application/json"}
    )
    started = time.monotonic()
    with urllib.request.urlopen(request, timeout=120) as res:
        answer = json.loads(res.read())
    text = (answer["choices"][0]["message"].get("content") or "").strip()
    return {"text": text, "ms": round((time.monotonic() - started) * 1000)}


def main() -> None:
    args = sys.argv[1:]
    label = args[0]
    constrain = "--grammar" in args
    repeat = int(args[args.index("--repeat") + 1]) if "--repeat" in args else 1
    url = args[args.index("--url") + 1] if "--url" in args else "http://127.0.0.1:8766"
    results = json.loads(RESULTS.read_text())
    cases = results["translate/qwen-mt-flash"]["runs"][::3]
    runs = []
    for case in cases:
        names = re.findall(r"⟦([^⟦⟧]*)⟧", case["source"])
        for _ in range(repeat):
            got = {lang: translate(url, case["source"], name, names, constrain) for lang, name in TARGETS.items()}
            found = {lang: problems(g["text"], names) for lang, g in got.items()}
            runs.append({
                "text": case["text"],
                "source": case["source"],
                # 1 言語ずつ順に聞いた時間の合計（アプリは同時に聞くので、目安は 1 言語の時間）
                "ms": sum(g["ms"] for g in got.values()),
                "msEach": {lang: g["ms"] for lang, g in got.items()},
                "ok": not any(found.values()),
                "translations": {lang: g["text"] for lang, g in got.items()},
                "problems": found,
            })
    each = sorted(ms for r in runs for ms in r["msEach"].values())
    summary = {
        "runs": len(runs),
        "ok": sum(r["ok"] for r in runs),
        "en": sum(not r["problems"]["en"] for r in runs),
        "ru": sum(not r["problems"]["ru"] for r in runs),
        "medianMsPerLanguage": each[len(each) // 2],
        "p90MsPerLanguage": each[len(each) * 9 // 10],
    }
    name = f"translate/local-{label}" + ("+grammar" if constrain else "")
    print(f"{name}: {json.dumps(summary)}", flush=True)
    results = json.loads(RESULTS.read_text())
    results[name] = {"summary": summary, "runs": runs}
    RESULTS.write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
